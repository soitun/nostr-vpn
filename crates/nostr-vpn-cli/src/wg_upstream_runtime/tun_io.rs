/// Spin up a userspace WG runtime over a POSIX `TunSocket` (Linux tun
/// or macOS utun). Builds the platform-specific reader+writer tasks
/// here so `nostr-vpn-core` doesn't need the boringtun `device`
/// feature (which doesn't compile on iOS/Android).
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub async fn start_wg_runtime_with_posix_tun(
    config: &WireGuardExitConfig,
    tun: Arc<TunSocket>,
) -> Result<WgUpstreamRuntime> {
    let (in_tx, in_rx) = mpsc::channel::<Vec<u8>>(WG_TUN_CHANNEL_CAPACITY);
    let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>(WG_TUN_CHANNEL_CAPACITY);
    let reader = spawn_posix_tun_reader(tun.clone(), in_tx);
    let writer = spawn_posix_tun_writer(tun, out_rx);
    WgUpstreamRuntime::start_with_io(config, Some((in_rx, out_tx)), Some((reader, writer))).await
}

/// Same idea for Windows WinTun.
#[cfg(target_os = "windows")]
pub async fn start_wg_runtime_with_wintun(
    config: &WireGuardExitConfig,
    session: Arc<WintunSession>,
) -> Result<WgUpstreamRuntime> {
    let (in_tx, in_rx) = mpsc::channel::<Vec<u8>>(WG_TUN_CHANNEL_CAPACITY);
    let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>(WG_TUN_CHANNEL_CAPACITY);
    let reader = spawn_wintun_reader(session.clone(), in_tx);
    let writer = spawn_wintun_writer(session, out_rx);
    WgUpstreamRuntime::start_with_io(config, Some((in_rx, out_tx)), Some((reader, writer))).await
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn spawn_posix_tun_reader(tun: Arc<TunSocket>, tun_tx: mpsc::Sender<Vec<u8>>) -> JoinHandle<()> {
    use std::os::unix::io::{AsRawFd, RawFd};
    use tokio::io::Interest;
    use tokio::io::unix::AsyncFd;

    struct BorrowedFd(RawFd);
    impl AsRawFd for BorrowedFd {
        fn as_raw_fd(&self) -> RawFd {
            self.0
        }
    }

    tokio::spawn(async move {
        let async_fd = match AsyncFd::with_interest(BorrowedFd(tun.as_raw_fd()), Interest::READABLE)
        {
            Ok(fd) => fd,
            Err(error) => {
                tracing::warn!(?error, "wg-upstream: failed to register tun fd");
                return;
            }
        };
        let mut buf = vec![0u8; MAX_WG_PACKET];
        loop {
            let mut guard = match async_fd.readable().await {
                Ok(g) => g,
                Err(error) => {
                    tracing::warn!(?error, "wg-upstream: tun reactor error");
                    return;
                }
            };
            match tun.read(&mut buf) {
                Ok([]) => guard.clear_ready(),
                Ok(packet) => {
                    let bytes = packet.to_vec();
                    if tun_tx.send(bytes).await.is_err() {
                        return;
                    }
                }
                Err(_) => guard.clear_ready(),
            }
        }
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn spawn_posix_tun_writer(tun: Arc<TunSocket>, mut rx: mpsc::Receiver<Vec<u8>>) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(packet) = rx.recv().await {
            match packet.first().map(|byte| byte >> 4) {
                Some(4) => {
                    let _ = tun.write4(&packet);
                }
                Some(6) => {
                    let _ = tun.write6(&packet);
                }
                _ => {}
            }
        }
    })
}

#[cfg(target_os = "windows")]
fn spawn_wintun_reader(
    session: Arc<WintunSession>,
    tun_tx: mpsc::Sender<Vec<u8>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        struct ShutdownOnDrop(Arc<WintunSession>);

        impl Drop for ShutdownOnDrop {
            fn drop(&mut self) {
                let _ = self.0.shutdown();
            }
        }

        let _shutdown_on_drop = ShutdownOnDrop(session.clone());
        let reader = tokio::task::spawn_blocking(move || {
            loop {
                let packet = match session.receive_blocking() {
                    Ok(packet) => packet,
                    Err(error) => {
                        tracing::warn!(?error, "wg-upstream: wintun receive failed");
                        return;
                    }
                };
                let bytes = packet.bytes().to_vec();
                drop(packet);
                if tun_tx.blocking_send(bytes).is_err() {
                    return;
                }

                for _ in 1..WG_WINTUN_READ_BURST {
                    match session.try_receive() {
                        Ok(Some(packet)) => {
                            let bytes = packet.bytes().to_vec();
                            drop(packet);
                            if tun_tx.blocking_send(bytes).is_err() {
                                return;
                            }
                        }
                        Ok(None) => break,
                        Err(error) => {
                            tracing::warn!(?error, "wg-upstream: wintun receive failed");
                            return;
                        }
                    }
                }
            }
        });
        let _ = reader.await;
    })
}

#[cfg(target_os = "windows")]
fn spawn_wintun_writer(
    session: Arc<WintunSession>,
    mut rx: mpsc::Receiver<Vec<u8>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(packet) = rx.recv().await {
            let Ok(size) = u16::try_from(packet.len()) else {
                tracing::warn!(
                    "wg-upstream: wintun packet too large to send ({} bytes)",
                    packet.len()
                );
                continue;
            };
            match session.allocate_send_packet(size) {
                Ok(mut outbound) => {
                    outbound.bytes_mut().copy_from_slice(&packet);
                    session.send_packet(outbound);
                }
                Err(error) => {
                    tracing::warn!(?error, "wg-upstream: wintun allocate_send_packet failed");
                }
            }
        }
    })
}
