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
    packet_tx: mpsc::Sender<Vec<Vec<u8>>>,
) -> ThreadJoinHandle<()> {
    thread::spawn(move || {
        let debug_packets = windows_fips_packet_debug_enabled();
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
            let payload = packet.bytes().to_vec();
            drop(packet);
            if debug_packets {
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
                        if debug_packets {
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
            if packet_tx.blocking_send(batch).is_err() {
                break;
            }
        }
    })
}

#[cfg(target_os = "windows")]
fn spawn_windows_fips_mesh_recv_task(
    mesh: Arc<FipsPrivateMeshRuntime>,
    session: Arc<Session>,
    event_tx: mpsc::Sender<FipsPrivateMeshEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut messages = Vec::with_capacity(WINDOWS_FIPS_TUN_WRITE_BURST);
        let mut events = Vec::with_capacity(WINDOWS_FIPS_TUN_WRITE_BURST);
        let mut packets = Vec::with_capacity(WINDOWS_FIPS_TUN_WRITE_BURST);
        loop {
            match mesh
                .recv_mesh_event_batch_into(
                    &mut messages,
                    &mut events,
                    WINDOWS_FIPS_TUN_WRITE_BURST,
                )
                .await
            {
                Ok(Some(_)) => {
                    for event in events.drain(..) {
                        match event {
                            FipsPrivateMeshEvent::Packet(packet) => {
                                packets.push(packet.into_vec());
                            }
                            event => {
                                write_windows_fips_packet_batch(&session, &mut packets);
                                if event_tx.send(event).await.is_err() {
                                    return;
                                }
                            }
                        }
                    }
                    write_windows_fips_packet_batch(&session, &mut packets);
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

#[cfg(target_os = "windows")]
fn write_windows_fips_packet_batch(session: &Arc<Session>, packets: &mut Vec<Vec<u8>>) {
    if packets.is_empty() {
        return;
    }
    let debug_packets = windows_fips_packet_debug_enabled();
    if debug_packets {
        for packet in packets.iter() {
            eprintln!(
                "fips: Windows mesh -> Wintun {} bytes {}",
                packet.len(),
                describe_ip_packet(packet)
            );
        }
    }
    if let Err(error) = crate::windows_tunnel::write_tunnel_packets(session, packets) {
        eprintln!("fips: failed to write Windows tunnel packet: {error}");
    }
    packets.clear();
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
