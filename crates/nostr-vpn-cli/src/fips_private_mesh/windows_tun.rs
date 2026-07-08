#[cfg(target_os = "windows")]
fn start_windows_fips_wintun(
    config: &FipsPrivateTunnelConfig,
) -> Result<(Arc<Session>, String, u32)> {
    let wintun = load_wintun()?;
    let adapter = Adapter::open(&wintun, &config.iface)
        .or_else(|_| Adapter::create(&wintun, &config.iface, "NostrVPN", None))
        .with_context(|| format!("failed to open or create wintun adapter {}", config.iface))?;
    adapter
        .set_mtu(config.mesh_mtu.tunnel as usize)
        .with_context(|| format!("failed to set MTU on wintun adapter {}", config.iface))?;
    let parsed_address = crate::windows_tunnel::windows_interface_address(&config.local_address)?;
    adapter
        .set_network_addresses_tuple(
            parsed_address.address.into(),
            parsed_address.mask.into(),
            None,
        )
        .with_context(|| format!("failed to set address on wintun adapter {}", config.iface))?;
    let interface_index = adapter
        .get_adapter_index()
        .with_context(|| format!("failed to resolve interface index for {}", config.iface))?;
    let session = Arc::new(
        adapter
            .start_session(MAX_RING_CAPACITY)
            .with_context(|| format!("failed to start wintun session for {}", config.iface))?,
    );
    Ok((session, config.iface.clone(), interface_index))
}

#[cfg(target_os = "windows")]
fn spawn_windows_fips_tun_read_thread(
    stop: Arc<AtomicBool>,
    session: Arc<Session>,
    mesh: Arc<FipsPrivateMeshRuntime>,
) -> ThreadJoinHandle<()> {
    thread::spawn(move || {
        let pipeline_profile_enabled = crate::pipeline_profile::enabled();
        let packet_debug = windows_fips_packet_debug_enabled();
        let mut send_runs = Vec::new();
        while !stop.load(Ordering::Relaxed) {
            let packet = match session.receive_blocking() {
                Ok(packet) => packet,
                Err(error) => {
                    if !stop.load(Ordering::Relaxed) {
                        eprintln!("fips: Windows Wintun receive failed: {error}");
                    }
                    break;
                }
            };
            let mut batch = Vec::with_capacity(WINDOWS_FIPS_TUN_READ_BURST);
            let mut batch_bytes = 0usize;
            {
                let _t =
                    crate::pipeline_profile::Timer::start(crate::pipeline_profile::Stage::TunRead);
                let payload = packet.bytes().to_vec();
                drop(packet);
                batch_bytes = batch_bytes.saturating_add(payload.len());
                if packet_debug {
                    eprintln!(
                        "fips: Windows Wintun read {} bytes {}",
                        payload.len(),
                        describe_ip_packet(&payload)
                    );
                }
                batch.push(payload);
                while batch.len() < WINDOWS_FIPS_TUN_READ_BURST {
                    match session.try_receive() {
                        Ok(Some(packet)) => {
                            let payload = packet.bytes().to_vec();
                            drop(packet);
                            batch_bytes = batch_bytes.saturating_add(payload.len());
                            if packet_debug {
                                eprintln!(
                                    "fips: Windows Wintun read {} bytes {}",
                                    payload.len(),
                                    describe_ip_packet(&payload)
                                );
                            }
                            batch.push(payload);
                        }
                        Ok(None) => break,
                        Err(error) => {
                            if !stop.load(Ordering::Relaxed) {
                                eprintln!("fips: Windows Wintun receive failed: {error}");
                            }
                            return;
                        }
                    }
                }
            }
            if pipeline_profile_enabled {
                crate::pipeline_profile::record_tun_read_batch(
                    batch.len(),
                    batch_bytes,
                    WINDOWS_FIPS_TUN_READ_BURST,
                );
            }
            let packet_count = batch.len();
            crate::pipeline_profile::record_mesh_send_bulk_turn(0, packet_count);
            let send_result = {
                let _t = crate::pipeline_profile::Timer::start(
                    crate::pipeline_profile::Stage::MeshSend,
                );
                mesh.blocking_send_tunnel_packet_batch_owned_with_capacity(
                    batch,
                    WINDOWS_FIPS_TUN_READ_BURST,
                    &mut send_runs,
                )
            };
            match send_result {
                Ok(sent) if sent == packet_count => {}
                Ok(_sent) if packet_debug => {
                    eprintln!("fips: Windows mesh route miss");
                }
                Ok(_sent) => {}
                Err(error) => {
                    eprintln!("fips: failed to send Windows tunnel packet: {error}");
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
    })
}

#[cfg(target_os = "windows")]
fn spawn_windows_fips_mesh_recv_task(
    stop: Arc<AtomicBool>,
    mesh: Arc<FipsPrivateMeshRuntime>,
    session: Arc<Session>,
    event_tx: mpsc::Sender<FipsPrivateMeshEvent>,
) -> JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        let recv_burst = WINDOWS_FIPS_TUN_WRITE_BURST;
        let mut packet_batch = DirectTunWriteBatch::with_capacity(recv_burst);
        let mut direct_rx = mesh.direct_endpoint_rx.cursor();
        let pipeline_profile_enabled = crate::pipeline_profile::enabled();
        while !stop.load(Ordering::Acquire) {
            packet_batch.clear();
            let received = mesh.recv_direct_endpoint_tun_batch_blocking(
                &mut direct_rx,
                recv_burst,
                &stop,
                &mut packet_batch,
                &event_tx,
            );
            match received {
                Ok(Some(drained)) => {
                    if let Err(error) =
                        mesh.finalize_direct_endpoint_tun_batch_blocking(&mut packet_batch)
                    {
                        packet_batch.clear();
                        eprintln!("fips: failed to finalize Windows tunnel packet batch: {error}");
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
                    write_windows_direct_endpoint_packet_batch(&session, &mut packet_batch);
                    if drained >= recv_burst {
                        std::thread::yield_now();
                    }
                }
                Ok(None) => {
                    if let Err(error) =
                        mesh.finalize_direct_endpoint_tun_batch_blocking(&mut packet_batch)
                    {
                        packet_batch.clear();
                        eprintln!("fips: failed to finalize Windows tunnel packet batch: {error}");
                    }
                    write_windows_direct_endpoint_packet_batch(&session, &mut packet_batch);
                    break;
                }
                Err(error) => {
                    eprintln!("fips: failed to receive Windows tunnel packet: {error}");
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    })
}

#[cfg(target_os = "windows")]
fn write_windows_direct_endpoint_packet_batch(
    session: &Arc<Session>,
    packet_batch: &mut DirectTunWriteBatch,
) {
    if packet_batch.is_empty() {
        return;
    }
    let _batch_timer =
        crate::pipeline_profile::Timer::start(crate::pipeline_profile::Stage::TunWriteBatch);
    let packet_debug = windows_fips_packet_debug_enabled();
    if packet_debug {
        for packet in packet_batch.run_slices() {
            eprintln!(
                "fips: Windows mesh -> Wintun {} bytes {}",
                packet.len(),
                describe_ip_packet(packet)
            );
        }
    }
    if crate::pipeline_profile::enabled() {
        crate::pipeline_profile::record_tun_write_packets(
            packet_batch.len(),
            packet_batch.bytes(),
        );
        for packet in packet_batch.run_slices() {
            crate::pipeline_profile::record_tun_write_frame(packet.len());
        }
    }
    let write_result = {
        let _t = crate::pipeline_profile::Timer::start(crate::pipeline_profile::Stage::TunWrite);
        crate::windows_tunnel::write_tunnel_packet_slices(session, packet_batch.run_slices())
    };
    if let Err(error) = write_result {
        eprintln!("fips: failed to write Windows tunnel packet: {error}");
    }
    packet_batch.clear();
}

#[cfg(target_os = "windows")]
fn windows_fips_packet_debug_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("NVPN_FIPS_PACKET_DEBUG").is_some())
}

#[cfg(target_os = "windows")]
fn describe_ip_packet(packet: &[u8]) -> String {
    match packet.first().map(|byte| byte >> 4) {
        Some(4) if packet.len() >= 20 => format!(
            "{} -> {}",
            std::net::Ipv4Addr::new(packet[12], packet[13], packet[14], packet[15]),
            std::net::Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19])
        ),
        Some(6) if packet.len() >= 40 => "IPv6".to_string(),
        Some(version) => format!("IPv{version} malformed"),
        None => "empty packet".to_string(),
    }
}
