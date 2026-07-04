fn spawn_tun_send_worker(
    tun: Arc<SystemTun>,
    mesh: Arc<FipsPrivateMeshRuntime>,
) -> FipsTunSendWorker {
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let thread = std::thread::Builder::new()
        .name("nvpn-fips-tun-send".to_string())
        .spawn(move || {
            let tun_fd = BorrowedTunFd::new(tun.as_raw_fd(), tun.vnet_hdr());
            let mut buf = vec![0_u8; tun.read_buffer_len()];
            let mut batch = Vec::with_capacity(FIPS_TUN_READ_BURST);
            let mut send_runs = Vec::new();
            let pipeline_profile_enabled = crate::pipeline_profile::enabled();
            while !thread_stop.load(Ordering::Acquire) {
                if !wait_fd_readable_blocking(tun_fd.as_raw_fd(), &thread_stop) {
                    break;
                }
                batch.clear();
                let mut drained = 0;
                let mut drained_bytes = 0usize;
                let mut sleep_after_error = false;
                loop {
                    let before_len = batch.len();
                    let read_result = {
                        let _t = crate::pipeline_profile::Timer::start(
                            crate::pipeline_profile::Stage::TunRead,
                        );
                        tun.read_packets_into(&mut buf, &mut batch)
                    };
                    match read_result {
                        Ok(0) => {
                            break;
                        }
                        Ok(packet_count) => {
                            debug_assert_eq!(batch.len(), before_len + packet_count);
                            if pipeline_profile_enabled {
                                for packet in &batch[before_len..] {
                                    drained_bytes =
                                        drained_bytes.saturating_add(packet.bytes.len());
                                }
                            }
                            drained += packet_count;
                            if drained >= FIPS_TUN_READ_BURST {
                                break;
                            }
                        }
                        Err(error) if temporary_tun_read_error(&error) => {
                            break;
                        }
                        Err(error) => {
                            eprintln!("fips: tunnel read failed: {error}");
                            sleep_after_error = true;
                            break;
                        }
                    }
                }

                if !batch.is_empty() {
                    if pipeline_profile_enabled {
                        crate::pipeline_profile::record_tun_read_batch(
                            batch.len(),
                            drained_bytes,
                            FIPS_TUN_READ_BURST,
                        );
                    }
                    let pending =
                        std::mem::replace(&mut batch, Vec::with_capacity(FIPS_TUN_READ_BURST));
                    send_mesh_packet_batch_blocking_or_log(
                        &mesh,
                        tun_fd,
                        pending,
                        &mut send_runs,
                        &thread_stop,
                    );
                }

                if sleep_after_error {
                    std::thread::sleep(Duration::from_millis(100));
                }

                if drained >= FIPS_TUN_READ_BURST {
                    std::thread::yield_now();
                }
            }
        })
        .expect("failed to spawn FIPS TUN send worker");
    FipsTunSendWorker { stop, thread }
}

async fn stop_tun_send_worker(worker: FipsTunSendWorker) {
    worker.stop.store(true, Ordering::Release);
    let _ = tokio::task::spawn_blocking(move || {
        let _ = worker.thread.join();
    })
    .await;
}
fn wait_fd_readable_blocking(fd: RawFd, stop: &AtomicBool) -> bool {
    while !stop.load(Ordering::Acquire) {
        let mut poll_fd = libc::pollfd {
            fd,
            events: libc::POLLIN,
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
        eprintln!("fips: tunnel read poll failed: {error}");
        return false;
    }
    false
}

fn send_mesh_packet_batch_blocking_or_log(
    mesh: &FipsPrivateMeshRuntime,
    tun_fd: BorrowedTunFd,
    packets: TunPipelineBatch,
    send_runs: &mut Vec<FipsEndpointSendRun>,
    stop: &AtomicBool,
) {
    let packet_count = packets.len();
    crate::pipeline_profile::record_mesh_send_bulk_turn(0, packet_count);
    let _t = crate::pipeline_profile::Timer::start(crate::pipeline_profile::Stage::MeshSend);
    let (local_packets, mesh_packets) =
        partition_local_tun_pipeline_packets(&mesh.local_tunnel_ips, packets);
    for packet in local_packets {
        write_packet_to_tun_blocking(tun_fd, &packet.bytes, stop);
    }
    let mesh_packet_count = mesh_packets.len();
    if let Err(error) =
        mesh.blocking_send_tun_pipeline_packet_turn(
            mesh_packets,
            mesh_packet_count,
            mesh_packet_count,
            send_runs,
        )
    {
        eprintln!("fips: failed to send tunnel packet: {error}");
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn partition_local_tun_pipeline_packets(
    local_tunnel_ips: &HashSet<IpAddr>,
    packets: TunPipelineBatch,
) -> (TunPipelineBatch, TunPipelineBatch) {
    if local_tunnel_ips.is_empty() {
        return (Vec::new(), packets);
    }

    let mut local_packets = Vec::new();
    let mut mesh_packets = Vec::with_capacity(packets.len());
    for packet in packets {
        if packet
            .destination
            .is_some_and(|destination| local_tunnel_ips.contains(&destination))
        {
            local_packets.push(packet);
        } else {
            mesh_packets.push(packet);
        }
    }
    (local_packets, mesh_packets)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn spawn_mesh_recv_worker(
    mesh: Arc<FipsPrivateMeshRuntime>,
    tun_fd: BorrowedTunFd,
    event_tx: mpsc::Sender<FipsPrivateMeshEvent>,
) -> FipsMeshRecvWorker {
    spawn_blocking_mesh_recv_worker(mesh, tun_fd, event_tx)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn stop_mesh_recv_worker(worker: FipsMeshRecvWorker, mesh: &FipsPrivateMeshRuntime) {
    worker.stop.store(true, Ordering::Release);
    mesh.wake_blocking_mesh_recv();
    let _ = tokio::task::spawn_blocking(move || {
        let _ = worker.thread.join();
    })
    .await;
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn spawn_blocking_mesh_recv_worker(
    mesh: Arc<FipsPrivateMeshRuntime>,
    tun_fd: BorrowedTunFd,
    event_tx: mpsc::Sender<FipsPrivateMeshEvent>,
) -> FipsMeshRecvWorker {
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let thread = std::thread::Builder::new()
        .name("nvpn-fips-mesh-recv".to_string())
        .spawn(move || {
            let recv_burst = FIPS_MESH_RECV_BURST;
            let mut packet_batch = DirectTunWriteBatch::with_capacity(recv_burst);
            #[cfg(target_os = "linux")]
            let mut vnet_write_preparer = LinuxVnetWritePreparer::new();
            let pipeline_profile_enabled = crate::pipeline_profile::enabled();
            while !thread_stop.load(Ordering::Acquire) {
                packet_batch.clear();
                let received = mesh.recv_direct_endpoint_tun_batch_blocking(
                    recv_burst,
                    &thread_stop,
                    &mut packet_batch,
                    Some(&event_tx),
                );
                match received {
                    Ok(Some(drained)) => {
                        if let Err(error) =
                            mesh.finalize_direct_endpoint_tun_batch_blocking(&mut packet_batch)
                        {
                            packet_batch.clear();
                            eprintln!("fips: failed to finalize tunnel packet batch: {error}");
                            std::thread::sleep(Duration::from_millis(100));
                            continue;
                        }
                        if pipeline_profile_enabled {
                            let packet_count = packet_batch.len();
                            let packet_bytes = packet_batch.bytes();
                            crate::pipeline_profile::record_mesh_recv_batch(
                                drained,
                                packet_count,
                                packet_bytes,
                                recv_burst,
                            );
                        }
                        if !packet_batch.is_empty() {
                            flush_direct_endpoint_packet_batch_to_tun_blocking(
                                tun_fd,
                                &mut packet_batch,
                                &thread_stop,
                                #[cfg(target_os = "linux")]
                                &mut vnet_write_preparer,
                            );
                        }
                        if drained >= recv_burst {
                            std::thread::yield_now();
                        }
                    }
                    Ok(None) => {
                        if let Err(error) =
                            mesh.finalize_direct_endpoint_tun_batch_blocking(&mut packet_batch)
                        {
                            packet_batch.clear();
                            eprintln!("fips: failed to finalize tunnel packet batch: {error}");
                        }
                        flush_direct_endpoint_packet_batch_to_tun_blocking(
                            tun_fd,
                            &mut packet_batch,
                            &thread_stop,
                            #[cfg(target_os = "linux")]
                            &mut vnet_write_preparer,
                        );
                        break;
                    }
                    Err(error) => {
                        if let Err(error) =
                            mesh.finalize_direct_endpoint_tun_batch_blocking(&mut packet_batch)
                        {
                            packet_batch.clear();
                            eprintln!("fips: failed to finalize tunnel packet batch: {error}");
                        }
                        flush_direct_endpoint_packet_batch_to_tun_blocking(
                            tun_fd,
                            &mut packet_batch,
                            &thread_stop,
                            #[cfg(target_os = "linux")]
                            &mut vnet_write_preparer,
                        );
                        eprintln!("fips: failed to receive tunnel packet: {error}");
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
            }
            })
        .expect("failed to spawn FIPS mesh receive worker");
    FipsMeshRecvWorker { stop, thread }
}
