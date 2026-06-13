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

#[cfg(target_os = "linux")]
enum SystemTun {
    Plain(TunSocket),
    Vnet(LinuxVnetTun),
}

#[cfg(target_os = "macos")]
struct SystemTun(TunSocket);

#[cfg(target_os = "linux")]
impl SystemTun {
    fn new(iface: &str) -> Result<Self> {
        if linux_vnet_tun_enabled() {
            LinuxVnetTun::new(iface).map(Self::Vnet)
        } else {
            TunSocket::new(iface).map(Self::Plain).map_err(Into::into)
        }
    }

    fn set_non_blocking(self) -> Result<Self> {
        match self {
            Self::Plain(tun) => tun.set_non_blocking().map(Self::Plain).map_err(Into::into),
            Self::Vnet(tun) => tun.set_non_blocking().map(Self::Vnet),
        }
    }

    fn name(&self) -> Result<String> {
        match self {
            Self::Plain(tun) => tun.name().map_err(Into::into),
            Self::Vnet(tun) => Ok(tun.name().to_string()),
        }
    }

    fn vnet_hdr(&self) -> bool {
        matches!(self, Self::Vnet(_))
    }

    fn read_buffer_len(&self) -> usize {
        match self {
            Self::Plain(_) => 65_535,
            Self::Vnet(_) => LINUX_VIRTIO_NET_HDR_LEN + 65_535,
        }
    }

    fn read_packets_into(
        &self,
        scratch: &mut [u8],
        batch: &mut TunPipelineBatch,
    ) -> io::Result<usize> {
        match self {
            Self::Plain(tun) => read_plain_tun_packets_into(tun, scratch, batch),
            Self::Vnet(tun) => tun.read_packets_into(scratch, batch),
        }
    }
}

#[cfg(target_os = "linux")]
impl AsRawFd for SystemTun {
    fn as_raw_fd(&self) -> RawFd {
        match self {
            Self::Plain(tun) => tun.as_raw_fd(),
            Self::Vnet(tun) => tun.as_raw_fd(),
        }
    }
}

#[cfg(target_os = "macos")]
impl SystemTun {
    fn new(iface: &str) -> Result<Self> {
        TunSocket::new(iface).map(Self).map_err(Into::into)
    }

    fn set_non_blocking(self) -> Result<Self> {
        self.0.set_non_blocking().map(Self).map_err(Into::into)
    }

    fn name(&self) -> Result<String> {
        self.0.name().map_err(Into::into)
    }

    fn vnet_hdr(&self) -> bool {
        false
    }

    fn read_buffer_len(&self) -> usize {
        65_535
    }

    fn read_packets_into(
        &self,
        scratch: &mut [u8],
        batch: &mut TunPipelineBatch,
    ) -> io::Result<usize> {
        read_plain_tun_packets_into(&self.0, scratch, batch)
    }
}

#[cfg(target_os = "macos")]
impl AsRawFd for SystemTun {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_plain_tun_packets_into(
    tun: &TunSocket,
    scratch: &mut [u8],
    batch: &mut TunPipelineBatch,
) -> io::Result<usize> {
    match tun.read(scratch) {
        Ok([]) => Ok(0),
        Ok(packet) => {
            push_tun_pipeline_packet(batch, packet);
            Ok(1)
        }
        Err(error) => Err(tun_error_to_io(error)),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn push_tun_pipeline_packet(batch: &mut TunPipelineBatch, packet: &[u8]) {
    let mut bytes = packet.to_vec();
    nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(&mut bytes);
    batch.push(TunPipelinePacket::new(bytes));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn tun_error_to_io(error: boringtun::device::Error) -> io::Error {
    match error {
        boringtun::device::Error::IfaceRead(source) => source,
        error => io::Error::other(error),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn spawn_tun_read_task(
    tun: Arc<SystemTun>,
    tun_fd: Arc<AsyncFd<BorrowedTunFd>>,
    packet_tx: TunPipelineQueueTx,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut buf = vec![0_u8; tun.read_buffer_len()];
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
                        // 0-byte read on a readable fd means "no packet right now";
                        // clear ready so the next readable().await blocks on the
                        // kernel instead of busy-looping.
                        guard.clear_ready();
                        break;
                    }
                    Ok(packet_count) => {
                        debug_assert_eq!(batch.len(), before_len + packet_count);
                        for packet in &batch[before_len..] {
                            drained_bytes = drained_bytes.saturating_add(packet.bytes.len());
                        }
                        drained += packet_count;
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
                crate::pipeline_profile::record_tun_read_batch(
                    batch.len(),
                    drained_bytes,
                    FIPS_TUN_READ_BURST,
                );
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
        let mut packet_batch = Vec::with_capacity(FIPS_MESH_RECV_BURST);
        loop {
            match mesh
                .recv_mesh_event_batch_into(&mut messages, &mut events, FIPS_MESH_RECV_BURST)
                .await
            {
                Ok(Some(drained)) => {
                    let (packet_count, packet_bytes) = mesh_event_packet_stats(&events);
                    crate::pipeline_profile::record_mesh_recv_batch(
                        drained,
                        packet_count,
                        packet_bytes,
                        FIPS_MESH_RECV_BURST,
                    );
                    packet_batch.clear();
                    for event in events.drain(..) {
                        if !forward_mesh_event_to_tun_batched(
                            event,
                            &tun_fd,
                            &event_tx,
                            &mut packet_batch,
                        )
                        .await
                        {
                            return;
                        }
                    }
                    flush_mesh_packet_batch_to_tun(&tun_fd, &mut packet_batch).await;

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
        let tun_fd = *tun_fd.get_ref();
        let mut packet_batch = Vec::with_capacity(FIPS_MESH_RECV_BURST);
        while !thread_stop.load(Ordering::Acquire) {
            let mut packet_count = 0usize;
            let mut packet_bytes = 0usize;
            packet_batch.clear();
            match mesh.recv_mesh_event_batch_blocking_for_each(
                FIPS_MESH_RECV_BURST,
                &thread_stop,
                |event| {
                    if let FipsPrivateMeshEvent::Packet(packet) = &event {
                        packet_count += 1;
                        packet_bytes = packet_bytes.saturating_add(packet.len());
                    }
                    !thread_stop.load(Ordering::Acquire)
                        && forward_mesh_event_to_tun_blocking_batched(
                            event,
                            tun_fd,
                            &event_tx,
                            &thread_stop,
                            &mut packet_batch,
                        )
                },
            ) {
                Ok(Some(drained)) => {
                    crate::pipeline_profile::record_mesh_recv_batch(
                        drained,
                        packet_count,
                        packet_bytes,
                        FIPS_MESH_RECV_BURST,
                    );
                    flush_mesh_packet_batch_to_tun_blocking(
                        tun_fd,
                        &mut packet_batch,
                        &thread_stop,
                    );
                }
                Ok(None) => {
                    flush_mesh_packet_batch_to_tun_blocking(
                        tun_fd,
                        &mut packet_batch,
                        &thread_stop,
                    );
                    break;
                }
                Err(error) => {
                    flush_mesh_packet_batch_to_tun_blocking(
                        tun_fd,
                        &mut packet_batch,
                        &thread_stop,
                    );
                    eprintln!("fips: failed to receive tunnel packet: {error}");
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    });
    FipsMeshRecvWorker::Blocking { stop, thread }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn mesh_event_packet_stats(events: &[FipsPrivateMeshEvent]) -> (usize, usize) {
    let mut packets = 0usize;
    let mut bytes = 0usize;
    for event in events {
        if let FipsPrivateMeshEvent::Packet(packet) = event {
            packets += 1;
            bytes = bytes.saturating_add(packet.len());
        }
    }
    (packets, bytes)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn forward_mesh_event_to_tun_batched(
    event: FipsPrivateMeshEvent,
    tun_fd: &AsyncFd<BorrowedTunFd>,
    event_tx: &mpsc::Sender<FipsPrivateMeshEvent>,
    packet_batch: &mut Vec<Vec<u8>>,
) -> bool {
    match event {
        FipsPrivateMeshEvent::Packet(packet) => {
            push_mesh_packet_for_tun(packet, packet_batch);
            true
        }
        event => {
            flush_mesh_packet_batch_to_tun(tun_fd, packet_batch).await;
            event_tx.send(event).await.is_ok()
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn cooperate_after_mesh_recv_packet() {
    // Endpoint recv can stay immediately ready while a peer sends bulk data,
    // and the utun write itself is synchronous. Count each packet against
    // Tokio's cooperative budget so timers/control traffic get scheduler time.
    tokio::task::coop::consume_budget().await;
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn push_mesh_packet_for_tun(mut packet: Vec<u8>, packet_batch: &mut Vec<Vec<u8>>) {
    nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(&mut packet);
    packet_batch.push(packet);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn flush_mesh_packet_batch_to_tun(
    tun_fd: &AsyncFd<BorrowedTunFd>,
    packet_batch: &mut Vec<Vec<u8>>,
) {
    if packet_batch.is_empty() {
        return;
    }

    #[cfg(target_os = "linux")]
    if tun_fd.get_ref().vnet_hdr {
        let packet_count = packet_batch.len();
        write_linux_vnet_packet_batch_to_tun(tun_fd, packet_batch).await;
        for _ in 0..packet_count {
            cooperate_after_mesh_recv_packet().await;
        }
        return;
    }

    for packet in packet_batch.drain(..) {
        // Hot path. Write to TUN inline and DON'T forward Packet events
        // upstream: the control-loop consumer discards packet events. The
        // raw fd write below still waits on utun writability instead of
        // silently dropping `EWOULDBLOCK` like boringtun's helper does.
        write_packet_to_tun(tun_fd, &packet).await;
        cooperate_after_mesh_recv_packet().await;
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn forward_mesh_event_to_tun_blocking_batched(
    event: FipsPrivateMeshEvent,
    tun_fd: BorrowedTunFd,
    event_tx: &mpsc::Sender<FipsPrivateMeshEvent>,
    stop: &AtomicBool,
    packet_batch: &mut Vec<Vec<u8>>,
) -> bool {
    match event {
        FipsPrivateMeshEvent::Packet(packet) => {
            push_mesh_packet_for_tun(packet, packet_batch);
            true
        }
        event => {
            flush_mesh_packet_batch_to_tun_blocking(tun_fd, packet_batch, stop);
            event_tx.blocking_send(event).is_ok()
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn flush_mesh_packet_batch_to_tun_blocking(
    tun_fd: BorrowedTunFd,
    packet_batch: &mut Vec<Vec<u8>>,
    stop: &AtomicBool,
) {
    if packet_batch.is_empty() {
        return;
    }

    #[cfg(target_os = "linux")]
    if tun_fd.vnet_hdr {
        write_linux_vnet_packet_batch_to_tun_blocking(tun_fd, packet_batch, stop);
        return;
    }

    for packet in packet_batch.drain(..) {
        write_packet_to_tun_blocking(tun_fd, &packet, stop);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn write_packet_to_tun(tun_fd: &AsyncFd<BorrowedTunFd>, packet: &[u8]) {
    let Some(address_family) = tunnel_packet_address_family(packet) else {
        return;
    };

    let _t = crate::pipeline_profile::Timer::start(crate::pipeline_profile::Stage::TunWrite);
    loop {
        match raw_write_packet_to_tun(tun_fd.get_ref(), packet, address_family) {
            Ok(()) => {
                crate::pipeline_profile::record_tun_write_packet(packet.len());
                crate::pipeline_profile::record_tun_write_frame(packet.len());
                return;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                crate::pipeline_profile::record_tun_write_would_block();
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
fn write_packet_to_tun_blocking(fd: BorrowedTunFd, packet: &[u8], stop: &AtomicBool) {
    let Some(address_family) = tunnel_packet_address_family(packet) else {
        return;
    };

    let _t = crate::pipeline_profile::Timer::start(crate::pipeline_profile::Stage::TunWrite);
    loop {
        if stop.load(Ordering::Acquire) {
            return;
        }
        match raw_write_packet_to_tun(&fd, packet, address_family) {
            Ok(()) => {
                crate::pipeline_profile::record_tun_write_packet(packet.len());
                crate::pipeline_profile::record_tun_write_frame(packet.len());
                return;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                crate::pipeline_profile::record_tun_write_would_block();
                if !wait_fd_writable_blocking(fd.as_raw_fd(), stop) {
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

#[cfg(target_os = "linux")]
async fn write_linux_vnet_packet_batch_to_tun(
    tun_fd: &AsyncFd<BorrowedTunFd>,
    packets: &mut Vec<Vec<u8>>,
) {
    let packet_count = packets.len();
    let packet_bytes = packets
        .iter()
        .fold(0usize, |total, packet| total.saturating_add(packet.len()));
    let frames = linux_vnet_prepare_write_frames(packets);
    packets.clear();
    crate::pipeline_profile::record_tun_write_packets(packet_count, packet_bytes);
    for frame in frames {
        write_linux_vnet_frame_to_tun(tun_fd, &frame).await;
    }
}

#[cfg(target_os = "linux")]
async fn write_linux_vnet_frame_to_tun(tun_fd: &AsyncFd<BorrowedTunFd>, frame: &[u8]) {
    let _t = crate::pipeline_profile::Timer::start(crate::pipeline_profile::Stage::TunWrite);
    loop {
        match raw_write_linux_vnet_frame_to_tun(tun_fd.get_ref(), frame) {
            Ok(()) => {
                crate::pipeline_profile::record_tun_write_frame(frame.len());
                return;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                crate::pipeline_profile::record_tun_write_would_block();
                match tun_fd.writable().await {
                    Ok(mut guard) => guard.clear_ready(),
                    Err(error) => {
                        eprintln!("fips: tunnel write readiness failed: {error}");
                        return;
                    }
                }
            }
            Err(error) => {
                eprintln!("fips: tunnel write failed: {error}");
                return;
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn write_linux_vnet_packet_batch_to_tun_blocking(
    tun_fd: BorrowedTunFd,
    packets: &mut Vec<Vec<u8>>,
    stop: &AtomicBool,
) {
    let packet_count = packets.len();
    let packet_bytes = packets
        .iter()
        .fold(0usize, |total, packet| total.saturating_add(packet.len()));
    let frames = linux_vnet_prepare_write_frames(packets);
    packets.clear();
    crate::pipeline_profile::record_tun_write_packets(packet_count, packet_bytes);
    for frame in frames {
        write_linux_vnet_frame_to_tun_blocking(tun_fd, &frame, stop);
    }
}

#[cfg(target_os = "linux")]
fn write_linux_vnet_frame_to_tun_blocking(
    tun_fd: BorrowedTunFd,
    frame: &[u8],
    stop: &AtomicBool,
) {
    let _t = crate::pipeline_profile::Timer::start(crate::pipeline_profile::Stage::TunWrite);
    loop {
        if stop.load(Ordering::Acquire) {
            return;
        }
        match raw_write_linux_vnet_frame_to_tun(&tun_fd, frame) {
            Ok(()) => {
                crate::pipeline_profile::record_tun_write_frame(frame.len());
                return;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                crate::pipeline_profile::record_tun_write_would_block();
                if !wait_fd_writable_blocking(tun_fd.as_raw_fd(), stop) {
                    return;
                }
            }
            Err(error) => {
                eprintln!("fips: tunnel write failed: {error}");
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
fn raw_write_packet_to_tun(
    tun_fd: &BorrowedTunFd,
    packet: &[u8],
    address_family: u8,
) -> io::Result<()> {
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
    let written = unsafe {
        libc::writev(
            tun_fd.as_raw_fd(),
            iov.as_ptr(),
            iov.len() as libc::c_int,
        )
    };
    raw_tun_write_result(written, header.len() + packet.len())
}

#[cfg(target_os = "linux")]
fn raw_write_packet_to_tun(
    tun_fd: &BorrowedTunFd,
    packet: &[u8],
    _address_family: u8,
) -> io::Result<()> {
    if tun_fd.vnet_hdr {
        let header = [0_u8; LINUX_VIRTIO_NET_HDR_LEN];
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
        let written = unsafe {
            libc::writev(
                tun_fd.as_raw_fd(),
                iov.as_ptr(),
                iov.len() as libc::c_int,
            )
        };
        raw_tun_write_result(written, header.len() + packet.len())
    } else {
        let written = unsafe {
            libc::write(
                tun_fd.as_raw_fd(),
                packet.as_ptr().cast::<libc::c_void>(),
                packet.len(),
            )
        };
        raw_tun_write_result(written, packet.len())
    }
}

#[cfg(target_os = "linux")]
fn raw_write_linux_vnet_frame_to_tun(tun_fd: &BorrowedTunFd, frame: &[u8]) -> io::Result<()> {
    let written = unsafe {
        libc::write(
            tun_fd.as_raw_fd(),
            frame.as_ptr().cast::<libc::c_void>(),
            frame.len(),
        )
    };
    raw_tun_write_result(written, frame.len())
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
fn temporary_tun_read_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
    )
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
