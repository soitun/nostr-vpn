#[cfg(any(target_os = "linux", test))]
const LINUX_CAP_NET_ADMIN_BIT: u32 = 12;

#[cfg(target_os = "linux")]
fn ensure_linux_tun_permissions(iface: &str) -> Result<()> {
    if fs::metadata("/dev/net/tun").is_err() {
        return Err(anyhow!(linux_tun_setup_error(
            iface,
            "missing /dev/net/tun device"
        )));
    }

    if let Ok(status) = fs::read_to_string("/proc/self/status")
        && linux_cap_eff_has_net_admin(&status) == Some(false)
    {
        return Err(anyhow!(linux_tun_setup_error(
            iface,
            "current process lacks CAP_NET_ADMIN"
        )));
    }

    Ok(())
}

#[cfg(any(target_os = "linux", test))]
fn linux_cap_eff_has_net_admin(status: &str) -> Option<bool> {
    let value = status
        .lines()
        .find_map(|line| line.trim_start().strip_prefix("CapEff:"))?
        .trim();
    let caps = u64::from_str_radix(value, 16).ok()?;
    Some((caps & (1_u64 << LINUX_CAP_NET_ADMIN_BIT)) != 0)
}

#[cfg(any(target_os = "linux", test))]
fn linux_tun_setup_error(iface: &str, reason: &str) -> String {
    format!(
        "Linux tunnel setup requires CAP_NET_ADMIN and /dev/net/tun before FIPS can create {iface}: {reason}. For a foreground session run `sudo nvpn start --connect` or `sudo nvpn connect`; for unattended use install/start the system service. In Docker add `--cap-add NET_ADMIN --device /dev/net/tun`."
    )
}

#[cfg(target_os = "linux")]
fn fips_tun_create_context(iface: &str) -> String {
    linux_tun_setup_error(iface, "kernel rejected TUN creation")
}

#[cfg(target_os = "macos")]
fn fips_tun_create_context(iface: &str) -> String {
    format!("failed to create FIPS tunnel {iface}")
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn spawn_tun_read_task(
    tun: Arc<TunSocket>,
    tun_fd: Arc<AsyncFd<BorrowedTunFd>>,
    packet_tx: TunPipelineQueueTx,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut buf = vec![0_u8; 65_535];
        let mut batch = Vec::with_capacity(FIPS_TUN_READ_BURST);
        loop {
            let mut guard = match tun_fd.readable().await {
                Ok(guard) => guard,
                Err(error) => {
                    eprintln!("fips: tun reactor await failed: {error}");
                    return;
                }
            };

            batch.clear();
            let mut drained = 0;
            let mut sleep_after_error = false;
            loop {
                let read_result = {
                    let _t = crate::pipeline_profile::Timer::start(
                        crate::pipeline_profile::Stage::TunRead,
                    );
                    tun.read(&mut buf)
                };
                match read_result {
                    Ok([]) => {
                        // 0-byte read on a readable fd means "no packet right now";
                        // clear ready so the next readable().await blocks on the
                        // kernel instead of busy-looping.
                        guard.clear_ready();
                        break;
                    }
                    Ok(packet) => {
                        let mut bytes = packet.to_vec();
                        nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(
                            &mut bytes,
                        );
                        batch.push(TunPipelinePacket::new(bytes));
                        drained += 1;
                        if drained >= FIPS_TUN_READ_BURST {
                            break;
                        }
                        // Keep reading while the fd is hot. BoringTun and
                        // wireguard-go both batch TUN-side work; without this
                        // bounded drain we pay a scheduler/channel round trip
                        // for every packet on the macOS laptop sender path.
                    }
                    Err(error) if temporary_tun_read_error(&error) => {
                        guard.clear_ready();
                        break;
                    }
                    Err(error) => {
                        eprintln!("fips: tunnel read failed: {error}");
                        guard.clear_ready();
                        sleep_after_error = true;
                        break;
                    }
                }
            }
            drop(guard);

            if !batch.is_empty() {
                let pending =
                    std::mem::replace(&mut batch, Vec::with_capacity(FIPS_TUN_READ_BURST));
                match submit_tun_packet_batch_to_mesh_queue(&packet_tx, pending) {
                    TunQueueSubmit::Enqueued | TunQueueSubmit::DroppedBulk => {}
                    TunQueueSubmit::Closed => return,
                }
            }

            if sleep_after_error {
                sleep(Duration::from_millis(100)).await;
            }

            if drained >= FIPS_TUN_READ_BURST {
                tokio::task::yield_now().await;
            }
        }
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn send_mesh_packet_batch_or_log(
    mesh: &FipsPrivateMeshRuntime,
    packets: &mut Vec<TunPipelinePacket>,
) {
    for packet in packets.iter() {
        crate::pipeline_profile::record_since(
            crate::pipeline_profile::Stage::TunToMeshQueueWait,
            packet.queued_at,
        );
    }

    let _t = crate::pipeline_profile::Timer::start(crate::pipeline_profile::Stage::MeshSend);
    if let Err(error) = mesh.send_tun_pipeline_packet_batch(packets).await {
        eprintln!("fips: failed to send tunnel packet: {error}");
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn fips_blocking_mesh_recv_enabled() -> bool {
    let value = std::env::var("NVPN_FIPS_BLOCKING_MESH_RECV").ok();
    fips_blocking_mesh_recv_enabled_from_env(value.as_deref())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn fips_blocking_mesh_recv_enabled_from_env(value: Option<&str>) -> bool {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return true;
    };

    !(value == "0"
        || value.eq_ignore_ascii_case("false")
        || value.eq_ignore_ascii_case("no")
        || value.eq_ignore_ascii_case("off")
        || value.eq_ignore_ascii_case("async"))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn spawn_mesh_recv_worker(
    mesh: Arc<FipsPrivateMeshRuntime>,
    tun_fd: Arc<AsyncFd<BorrowedTunFd>>,
    event_tx: mpsc::Sender<FipsPrivateMeshEvent>,
) -> FipsMeshRecvWorker {
    if fips_blocking_mesh_recv_enabled() {
        spawn_blocking_mesh_recv_worker(mesh, tun_fd, event_tx)
    } else {
        FipsMeshRecvWorker::Async(spawn_mesh_recv_task(mesh, tun_fd, event_tx))
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn stop_mesh_recv_worker(worker: FipsMeshRecvWorker, mesh: &FipsPrivateMeshRuntime) {
    match worker {
        FipsMeshRecvWorker::Async(handle) => {
            handle.abort();
            let _ = handle.await;
        }
        FipsMeshRecvWorker::Blocking { stop, thread } => {
            stop.store(true, Ordering::Release);
            mesh.wake_blocking_mesh_recv();
            let _ = tokio::task::spawn_blocking(move || thread.join()).await;
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn spawn_mesh_recv_task(
    mesh: Arc<FipsPrivateMeshRuntime>,
    tun_fd: Arc<AsyncFd<BorrowedTunFd>>,
    event_tx: mpsc::Sender<FipsPrivateMeshEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut messages = Vec::with_capacity(FIPS_MESH_RECV_BURST);
        let mut events = Vec::with_capacity(FIPS_MESH_RECV_BURST);
        loop {
            match mesh
                .recv_mesh_event_batch_into(&mut messages, &mut events, FIPS_MESH_RECV_BURST)
                .await
            {
                Ok(Some(drained)) => {
                    for event in events.drain(..) {
                        if !forward_mesh_event_to_tun_and_cooperate(event, &tun_fd, &event_tx).await
                        {
                            return;
                        }
                    }

                    if drained == FIPS_MESH_RECV_BURST {
                        tokio::task::yield_now().await;
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    eprintln!("fips: failed to receive tunnel packet: {error}");
                    sleep(Duration::from_millis(100)).await;
                }
            }
        }
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn spawn_blocking_mesh_recv_worker(
    mesh: Arc<FipsPrivateMeshRuntime>,
    tun_fd: Arc<AsyncFd<BorrowedTunFd>>,
    event_tx: mpsc::Sender<FipsPrivateMeshEvent>,
) -> FipsMeshRecvWorker {
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let thread = std::thread::spawn(move || {
        let tun_fd = tun_fd.get_ref().as_raw_fd();
        while !thread_stop.load(Ordering::Acquire) {
            match mesh.recv_mesh_event_batch_blocking_for_each(
                FIPS_MESH_RECV_BURST,
                &thread_stop,
                |event| {
                    !thread_stop.load(Ordering::Acquire)
                        && forward_mesh_event_to_tun_blocking(
                            event,
                            tun_fd,
                            &event_tx,
                            &thread_stop,
                        )
                },
            ) {
                Ok(Some(_drained)) => {}
                Ok(None) => break,
                Err(error) => {
                    eprintln!("fips: failed to receive tunnel packet: {error}");
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    });
    FipsMeshRecvWorker::Blocking { stop, thread }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn forward_mesh_event_to_tun_and_cooperate(
    event: FipsPrivateMeshEvent,
    tun_fd: &AsyncFd<BorrowedTunFd>,
    event_tx: &mpsc::Sender<FipsPrivateMeshEvent>,
) -> bool {
    let wrote_packet = matches!(&event, FipsPrivateMeshEvent::Packet(_));
    if !forward_mesh_event_to_tun(event, tun_fd, event_tx).await {
        return false;
    }
    if wrote_packet {
        cooperate_after_mesh_recv_packet().await;
    }
    true
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn cooperate_after_mesh_recv_packet() {
    // Endpoint recv can stay immediately ready while a peer sends bulk data,
    // and the utun write itself is synchronous. Count each packet against
    // Tokio's cooperative budget so timers/control traffic get scheduler time.
    tokio::task::coop::consume_budget().await;
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn forward_mesh_event_to_tun(
    event: FipsPrivateMeshEvent,
    tun_fd: &AsyncFd<BorrowedTunFd>,
    event_tx: &mpsc::Sender<FipsPrivateMeshEvent>,
) -> bool {
    match event {
        FipsPrivateMeshEvent::Packet(packet) => {
            // Hot path. Write to TUN inline and DON'T forward the Packet event
            // upstream: the control-loop consumer discards packet events. The
            // raw fd write below still waits on utun writability instead of
            // silently dropping `EWOULDBLOCK` like boringtun's helper does.
            let mut bytes = packet;
            nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(&mut bytes);
            write_packet_to_tun(tun_fd, &bytes).await;
            true
        }
        event => event_tx.send(event).await.is_ok(),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn forward_mesh_event_to_tun_blocking(
    event: FipsPrivateMeshEvent,
    tun_fd: RawFd,
    event_tx: &mpsc::Sender<FipsPrivateMeshEvent>,
    stop: &AtomicBool,
) -> bool {
    match event {
        FipsPrivateMeshEvent::Packet(mut packet) => {
            nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(&mut packet);
            write_packet_to_tun_blocking(tun_fd, &packet, stop);
            true
        }
        event => event_tx.blocking_send(event).is_ok(),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn write_packet_to_tun(tun_fd: &AsyncFd<BorrowedTunFd>, packet: &[u8]) {
    let Some(address_family) = tunnel_packet_address_family(packet) else {
        return;
    };

    let _t = crate::pipeline_profile::Timer::start(crate::pipeline_profile::Stage::TunWrite);
    loop {
        match raw_write_packet_to_tun(tun_fd.get_ref().as_raw_fd(), packet, address_family) {
            Ok(()) => return,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                match tun_fd.writable().await {
                    Ok(mut guard) => guard.clear_ready(),
                    Err(error) => {
                        eprintln!("fips: tunnel write reactor await failed: {error}");
                        return;
                    }
                }
            }
            Err(error) => {
                eprintln!("fips: failed to write tunnel packet: {error}");
                return;
            }
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn write_packet_to_tun_blocking(fd: RawFd, packet: &[u8], stop: &AtomicBool) {
    let Some(address_family) = tunnel_packet_address_family(packet) else {
        return;
    };

    let _t = crate::pipeline_profile::Timer::start(crate::pipeline_profile::Stage::TunWrite);
    loop {
        if stop.load(Ordering::Acquire) {
            return;
        }
        match raw_write_packet_to_tun(fd, packet, address_family) {
            Ok(()) => return,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if !wait_fd_writable_blocking(fd, stop) {
                    return;
                }
            }
            Err(error) => {
                eprintln!("fips: failed to write tunnel packet: {error}");
                return;
            }
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn wait_fd_writable_blocking(fd: RawFd, stop: &AtomicBool) -> bool {
    while !stop.load(Ordering::Acquire) {
        let mut poll_fd = libc::pollfd {
            fd,
            events: libc::POLLOUT,
            revents: 0,
        };
        let result = unsafe { libc::poll(&mut poll_fd, 1, 100) };
        if result > 0 {
            return true;
        }
        if result == 0 {
            continue;
        }
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            continue;
        }
        eprintln!("fips: tunnel write poll failed: {error}");
        return false;
    }
    false
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn tunnel_packet_address_family(packet: &[u8]) -> Option<u8> {
    match packet.first().map(|byte| byte >> 4) {
        #[cfg(target_os = "macos")]
        Some(4) => Some(2),
        #[cfg(target_os = "macos")]
        Some(6) => Some(30),
        #[cfg(target_os = "linux")]
        Some(4) | Some(6) => Some(0),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
fn raw_write_packet_to_tun(fd: RawFd, packet: &[u8], address_family: u8) -> io::Result<()> {
    let header = [0_u8, 0, 0, address_family];
    let iov = [
        libc::iovec {
            iov_base: header.as_ptr() as *mut libc::c_void,
            iov_len: header.len(),
        },
        libc::iovec {
            iov_base: packet.as_ptr() as *mut libc::c_void,
            iov_len: packet.len(),
        },
    ];
    let written = unsafe { libc::writev(fd, iov.as_ptr(), iov.len() as libc::c_int) };
    raw_tun_write_result(written, header.len() + packet.len())
}

#[cfg(target_os = "linux")]
fn raw_write_packet_to_tun(fd: RawFd, packet: &[u8], _address_family: u8) -> io::Result<()> {
    let written = unsafe { libc::write(fd, packet.as_ptr().cast::<libc::c_void>(), packet.len()) };
    raw_tun_write_result(written, packet.len())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn raw_tun_write_result(written: libc::ssize_t, expected: usize) -> io::Result<()> {
    if written < 0 {
        return Err(io::Error::last_os_error());
    }
    if written as usize == expected {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "short tunnel packet write",
        ))
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn temporary_tun_read_error(error: &TunError) -> bool {
    match error {
        TunError::IfaceRead(source) => matches!(
            source.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
        ),
        _ => false,
    }
}

#[cfg(target_os = "windows")]
pub(crate) struct FipsPrivateTunnelRuntime {
    iface: String,
    mesh: Arc<FipsPrivateMeshRuntime>,
    config: FipsPrivateTunnelConfig,
    session: Arc<Session>,
    stop: Arc<AtomicBool>,
    tun_read_thread: ThreadJoinHandle<()>,
    mesh_send_task: JoinHandle<()>,
    mesh_recv_task: JoinHandle<()>,
    event_rx: mpsc::Receiver<FipsPrivateMeshEvent>,
    interface_index: u32,
    route_targets: Vec<String>,
    /// Same shape as the macOS variant: a userspace WG upstream
    /// tunnel (boringtun + a *separate* WinTun adapter, distinct from
    /// the FIPS adapter above) that the daemon reconciles whenever
    /// `wireguard_exit` changes.
    wg_upstream: Option<crate::wg_upstream_runtime::DaemonWgUpstream>,
}
