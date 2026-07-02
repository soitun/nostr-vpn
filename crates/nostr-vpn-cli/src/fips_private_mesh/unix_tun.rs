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
    push_tun_pipeline_packet_owned(batch, copy_with_fips_endpoint_headroom(packet));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn push_tun_pipeline_packet_owned(batch: &mut TunPipelineBatch, mut bytes: Vec<u8>) {
    nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(&mut bytes);
    push_tun_pipeline_packet_owned_finalized(batch, bytes);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn push_tun_pipeline_packet_owned_finalized(batch: &mut TunPipelineBatch, bytes: Vec<u8>) {
    if fips_unix_packet_debug_enabled() {
        eprintln!(
            "fips: TUN -> mesh {} bytes {}",
            bytes.len(),
            describe_ip_packet(&bytes)
        );
    }
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
fn spawn_tun_send_worker(
    tun: Arc<SystemTun>,
    mesh: Arc<FipsPrivateMeshRuntime>,
) -> FipsTunSendWorker {
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let thread = std::thread::Builder::new()
        .name("nvpn-fips-tun-send".to_string())
        .spawn(move || {
            let tun_fd = tun.as_raw_fd();
            let mut buf = vec![0_u8; tun.read_buffer_len()];
            let mut batch = Vec::with_capacity(FIPS_TUN_READ_BURST);
            let mut send_runs = Vec::new();
            let pipeline_profile_enabled = crate::pipeline_profile::enabled();
            while !thread_stop.load(Ordering::Acquire) {
                if !wait_fd_readable_blocking(tun_fd, &thread_stop) {
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
                    send_mesh_packet_batch_blocking_or_log(&mesh, pending, &mut send_runs);
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn stop_tun_send_worker(worker: FipsTunSendWorker) {
    worker.stop.store(true, Ordering::Release);
    let _ = tokio::task::spawn_blocking(move || {
        let _ = worker.thread.join();
    })
    .await;
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn send_mesh_packet_batch_blocking_or_log(
    mesh: &FipsPrivateMeshRuntime,
    packets: TunPipelineBatch,
    send_runs: &mut Vec<FipsEndpointSendRun>,
) {
    let packet_count = packets.len();
    crate::pipeline_profile::record_mesh_send_bulk_turn(0, packet_count);
    let _t = crate::pipeline_profile::Timer::start(crate::pipeline_profile::Stage::MeshSend);
    if let Err(error) =
        mesh.blocking_send_tun_pipeline_packet_turn(packets, packet_count, packet_count, send_runs)
    {
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
        FipsMeshRecvWorker::Blocking { stop, threads } => {
            stop.store(true, Ordering::Release);
            mesh.wake_blocking_mesh_recv();
            for thread in threads {
                let _ = tokio::task::spawn_blocking(move || {
                    let _ = thread.join();
                })
                .await;
            }
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
        let recv_burst = fips_mesh_recv_burst();
        let mut messages = Vec::with_capacity(recv_burst);
        let mut events = Vec::with_capacity(recv_burst);
        let mut packet_batch = TunWriteBatch::with_capacity(recv_burst);
        #[cfg(target_os = "linux")]
        let mut vnet_write_preparer = LinuxVnetWritePreparer::new();
        let pipeline_profile_enabled = crate::pipeline_profile::enabled();
        loop {
            match mesh
                .recv_mesh_event_batch_into(&mut messages, &mut events, recv_burst)
                .await
            {
                Ok(Some(drained)) => {
                    if pipeline_profile_enabled {
                        let (packet_count, packet_bytes) = mesh_event_packet_stats(&events);
                        crate::pipeline_profile::record_mesh_recv_batch(
                            drained,
                            packet_count,
                            packet_bytes,
                            recv_burst,
                        );
                    }
                    packet_batch.clear();
                    for event in events.drain(..) {
                        if !forward_mesh_event_to_tun_batched(
                            event,
                            &tun_fd,
                            &event_tx,
                            &mut packet_batch,
                            #[cfg(target_os = "linux")]
                            &mut vnet_write_preparer,
                        )
                        .await
                        {
                            return;
                        }
                    }
                    flush_mesh_packet_batch_to_tun(
                        &tun_fd,
                        &mut packet_batch,
                        #[cfg(target_os = "linux")]
                        &mut vnet_write_preparer,
                    )
                    .await;

                    if drained == recv_burst {
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
    _event_tx: mpsc::Sender<FipsPrivateMeshEvent>,
) -> FipsMeshRecvWorker {
    let stop = Arc::new(AtomicBool::new(false));
    let tun_fd = *tun_fd.get_ref();
    let direct_tun_write_gate = Arc::new(Mutex::new(()));
    let lane_count = mesh
        .direct_endpoint_rx
        .as_ref()
        .map_or(1, |receivers| receivers.len())
        .max(1);
    let mut threads = Vec::with_capacity(lane_count);
    for lane in 0..lane_count {
        let mesh = Arc::clone(&mesh);
        let thread_stop = Arc::clone(&stop);
        let direct_tun_write_gate = Arc::clone(&direct_tun_write_gate);
        let thread = std::thread::Builder::new()
            .name(format!("nvpn-fips-mesh-recv-{lane}"))
            .spawn(move || {
            let recv_burst = fips_mesh_recv_burst();
            let mut packet_batch = DirectTunWriteBatch::with_capacity(recv_burst);
            #[cfg(target_os = "linux")]
            let mut vnet_write_preparer = LinuxVnetWritePreparer::new();
            let pipeline_profile_enabled = crate::pipeline_profile::enabled();
            while !thread_stop.load(Ordering::Acquire) {
                packet_batch.clear();
                let received = mesh.recv_direct_endpoint_tun_batch_blocking(
                    lane,
                    recv_burst,
                    &thread_stop,
                    &mut packet_batch,
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
                                &direct_tun_write_gate,
                                #[cfg(target_os = "linux")]
                                &mut vnet_write_preparer,
                            );
                        }
                        if drained == recv_burst {
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
                            &direct_tun_write_gate,
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
                            &direct_tun_write_gate,
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
        threads.push(thread);
    }
    FipsMeshRecvWorker::Blocking { stop, threads }
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
    packet_batch: &mut TunWriteBatch,
    #[cfg(target_os = "linux")] vnet_write_preparer: &mut LinuxVnetWritePreparer,
) -> bool {
    match event {
        FipsPrivateMeshEvent::Packet(packet) => {
            push_direct_packet_output_for_tun(packet, packet_batch);
            true
        }
        event => {
            flush_mesh_packet_batch_to_tun(
                tun_fd,
                packet_batch,
                #[cfg(target_os = "linux")]
                vnet_write_preparer,
            )
            .await;
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
fn push_direct_packet_output_for_tun(mut packet: FipsEndpointData, packet_batch: &mut TunWriteBatch) {
    nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(packet.as_mut_slice());
    if fips_unix_packet_debug_enabled() {
        eprintln!(
            "fips: mesh -> TUN {} bytes {}",
            packet.len(),
            describe_ip_packet(packet.as_slice())
        );
    }
    packet_batch.push(packet);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn flush_mesh_packet_batch_to_tun(
    tun_fd: &AsyncFd<BorrowedTunFd>,
    packet_batch: &mut TunWriteBatch,
    #[cfg(target_os = "linux")] vnet_write_preparer: &mut LinuxVnetWritePreparer,
) {
    if packet_batch.is_empty() {
        return;
    }

    #[cfg(target_os = "linux")]
    if tun_fd.get_ref().vnet_hdr {
        let packet_count = packet_batch.len();
        write_linux_vnet_packet_batch_to_tun(
            tun_fd,
            packet_batch,
            vnet_write_preparer,
        )
        .await;
        packet_batch.clear();
        for _ in 0..packet_count {
            cooperate_after_mesh_recv_packet().await;
        }
        return;
    }

    for packet in packet_batch.drain_packets() {
        // Hot path. Write to TUN inline and DON'T forward Packet events
        // upstream: the control-loop consumer discards packet events.
        write_packet_to_tun(tun_fd, packet.as_slice()).await;
        cooperate_after_mesh_recv_packet().await;
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn forward_mesh_event_to_tun_blocking_batched(
    event: FipsPrivateMeshEvent,
    tun_fd: BorrowedTunFd,
    event_tx: &mpsc::Sender<FipsPrivateMeshEvent>,
    stop: &AtomicBool,
    packet_batch: &mut TunWriteBatch,
    #[cfg(target_os = "linux")] vnet_write_preparer: &mut LinuxVnetWritePreparer,
) -> bool {
    match event {
        FipsPrivateMeshEvent::Packet(packet) => {
            push_direct_packet_output_for_tun(packet, packet_batch);
            true
        }
        event => {
            flush_mesh_packet_batch_to_tun_blocking(
                tun_fd,
                packet_batch,
                stop,
                #[cfg(target_os = "linux")]
                vnet_write_preparer,
            );
            event_tx.blocking_send(event).is_ok()
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn flush_mesh_packet_batch_to_tun_blocking(
    tun_fd: BorrowedTunFd,
    packet_batch: &mut TunWriteBatch,
    stop: &AtomicBool,
    #[cfg(target_os = "linux")] vnet_write_preparer: &mut LinuxVnetWritePreparer,
) {
    if packet_batch.is_empty() {
        return;
    }

    #[cfg(target_os = "linux")]
    if tun_fd.vnet_hdr {
        let packet_count = packet_batch.len();
        let packet_bytes = packet_batch.bytes();
        write_linux_vnet_packet_batch_to_tun_blocking(
            tun_fd,
            packet_batch,
            packet_count,
            packet_bytes,
            stop,
            vnet_write_preparer,
            None,
        );
        packet_batch.clear();
        return;
    }

    for packet in packet_batch.drain_packets() {
        write_packet_to_tun_blocking(tun_fd, packet.as_slice(), stop, None);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn flush_direct_endpoint_packet_batch_to_tun_blocking(
    tun_fd: BorrowedTunFd,
    packet_batch: &mut DirectTunWriteBatch,
    stop: &AtomicBool,
    direct_tun_write_gate: &Mutex<()>,
    #[cfg(target_os = "linux")] vnet_write_preparer: &mut LinuxVnetWritePreparer,
) {
    if packet_batch.is_empty() {
        return;
    }

    #[cfg(target_os = "linux")]
    if tun_fd.vnet_hdr {
        let packet_count = packet_batch.len();
        let packet_bytes = packet_batch.bytes();
        write_linux_vnet_packet_batch_to_tun_blocking(
            tun_fd,
            packet_batch,
            packet_count,
            packet_bytes,
            stop,
            vnet_write_preparer,
            Some(direct_tun_write_gate),
        );
        packet_batch.clear();
        return;
    }

    for packet in packet_batch.run_slices() {
        write_packet_to_tun_blocking(tun_fd, packet, stop, Some(direct_tun_write_gate));
    }
    packet_batch.clear();
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
fn write_packet_to_tun_blocking(
    fd: BorrowedTunFd,
    packet: &[u8],
    stop: &AtomicBool,
    write_gate: Option<&Mutex<()>>,
) {
    let Some(address_family) = tunnel_packet_address_family(packet) else {
        return;
    };

    let _t = crate::pipeline_profile::Timer::start(crate::pipeline_profile::Stage::TunWrite);
    loop {
        if stop.load(Ordering::Acquire) {
            return;
        }
        match raw_write_packet_to_tun_gated(&fd, packet, address_family, write_gate) {
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
    packets: &TunWriteBatch,
    preparer: &mut LinuxVnetWritePreparer,
) {
    let packet_count = packets.len();
    let packet_bytes = packets.bytes();
    preparer.prepare(packets);
    crate::pipeline_profile::record_tun_write_packets(packet_count, packet_bytes);
    for frame_index in 0..preparer.frames().len() {
        let frame = preparer.frames()[frame_index];
        write_linux_vnet_prepared_frame_to_tun(tun_fd, packets, preparer, frame).await;
    }
}

#[cfg(target_os = "linux")]
async fn write_linux_vnet_prepared_frame_to_tun<P: LinuxVnetPacketBatch + ?Sized>(
    tun_fd: &AsyncFd<BorrowedTunFd>,
    packets: &P,
    preparer: &mut LinuxVnetWritePreparer,
    frame: LinuxVnetPreparedWriteFrame,
) {
    match frame {
        LinuxVnetPreparedWriteFrame::RawPacket(packet_index) => {
            write_linux_vnet_raw_packet_to_tun(tun_fd, packets.packet_slice(packet_index)).await
        }
        LinuxVnetPreparedWriteFrame::Vectored(frame_index) => {
            write_linux_vnet_vectored_frame_to_tun(tun_fd, packets, preparer, frame_index).await
        }
    }
}

#[cfg(target_os = "linux")]
async fn write_linux_vnet_raw_packet_to_tun(tun_fd: &AsyncFd<BorrowedTunFd>, packet: &[u8]) {
    let _t = crate::pipeline_profile::Timer::start(crate::pipeline_profile::Stage::TunWrite);
    loop {
        match raw_write_packet_to_tun(tun_fd.get_ref(), packet, 0) {
            Ok(()) => {
                crate::pipeline_profile::record_tun_write_frame(
                    LINUX_VIRTIO_NET_HDR_LEN + packet.len(),
                );
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
async fn write_linux_vnet_vectored_frame_to_tun<P: LinuxVnetPacketBatch + ?Sized>(
    tun_fd: &AsyncFd<BorrowedTunFd>,
    packets: &P,
    preparer: &mut LinuxVnetWritePreparer,
    frame_index: usize,
) {
    let _t = crate::pipeline_profile::Timer::start(crate::pipeline_profile::Stage::TunWrite);
    loop {
        match preparer.write_vectored_frame_to_tun(tun_fd.get_ref(), packets, frame_index, None) {
            Ok(written) => {
                crate::pipeline_profile::record_tun_write_frame(written);
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
fn write_linux_vnet_packet_batch_to_tun_blocking<P: LinuxVnetPacketBatch + ?Sized>(
    tun_fd: BorrowedTunFd,
    packets: &P,
    packet_count: usize,
    packet_bytes: usize,
    stop: &AtomicBool,
    preparer: &mut LinuxVnetWritePreparer,
    write_gate: Option<&Mutex<()>>,
) {
    preparer.prepare(packets);
    crate::pipeline_profile::record_tun_write_packets(packet_count, packet_bytes);
    for frame_index in 0..preparer.frames().len() {
        let frame = preparer.frames()[frame_index];
        write_linux_vnet_prepared_frame_to_tun_blocking(
            tun_fd,
            packets,
            preparer,
            frame,
            stop,
            write_gate,
        );
    }
}

#[cfg(target_os = "linux")]
fn write_linux_vnet_prepared_frame_to_tun_blocking<P: LinuxVnetPacketBatch + ?Sized>(
    tun_fd: BorrowedTunFd,
    packets: &P,
    preparer: &mut LinuxVnetWritePreparer,
    frame: LinuxVnetPreparedWriteFrame,
    stop: &AtomicBool,
    write_gate: Option<&Mutex<()>>,
) {
    match frame {
        LinuxVnetPreparedWriteFrame::RawPacket(packet_index) => {
            write_linux_vnet_raw_packet_to_tun_blocking(
                tun_fd,
                packets.packet_slice(packet_index),
                stop,
                write_gate,
            )
        }
        LinuxVnetPreparedWriteFrame::Vectored(frame_index) => {
            write_linux_vnet_vectored_frame_to_tun_blocking(
                tun_fd,
                packets,
                preparer,
                frame_index,
                stop,
                write_gate,
            )
        }
    }
}

#[cfg(target_os = "linux")]
fn write_linux_vnet_raw_packet_to_tun_blocking(
    tun_fd: BorrowedTunFd,
    packet: &[u8],
    stop: &AtomicBool,
    write_gate: Option<&Mutex<()>>,
) {
    let _t = crate::pipeline_profile::Timer::start(crate::pipeline_profile::Stage::TunWrite);
    loop {
        if stop.load(Ordering::Acquire) {
            return;
        }
        match raw_write_packet_to_tun_gated(&tun_fd, packet, 0, write_gate) {
            Ok(()) => {
                crate::pipeline_profile::record_tun_write_frame(
                    LINUX_VIRTIO_NET_HDR_LEN + packet.len(),
                );
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

#[cfg(target_os = "linux")]
fn write_linux_vnet_frame_to_tun_blocking(
    tun_fd: BorrowedTunFd,
    frame: &[u8],
    stop: &AtomicBool,
    write_gate: Option<&Mutex<()>>,
) {
    let _t = crate::pipeline_profile::Timer::start(crate::pipeline_profile::Stage::TunWrite);
    loop {
        if stop.load(Ordering::Acquire) {
            return;
        }
        match raw_write_linux_vnet_frame_to_tun_gated(&tun_fd, frame, write_gate) {
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

#[cfg(target_os = "linux")]
fn write_linux_vnet_vectored_frame_to_tun_blocking<P: LinuxVnetPacketBatch + ?Sized>(
    tun_fd: BorrowedTunFd,
    packets: &P,
    preparer: &mut LinuxVnetWritePreparer,
    frame_index: usize,
    stop: &AtomicBool,
    write_gate: Option<&Mutex<()>>,
) {
    let _t = crate::pipeline_profile::Timer::start(crate::pipeline_profile::Stage::TunWrite);
    loop {
        if stop.load(Ordering::Acquire) {
            return;
        }
        match preparer.write_vectored_frame_to_tun(&tun_fd, packets, frame_index, write_gate) {
            Ok(written) => {
                crate::pipeline_profile::record_tun_write_frame(written);
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn with_tun_write_gate<T>(write_gate: Option<&Mutex<()>>, write: impl FnOnce() -> T) -> T {
    let Some(write_gate) = write_gate else {
        return write();
    };
    let _guard = write_gate
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    write()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn raw_write_packet_to_tun_gated(
    tun_fd: &BorrowedTunFd,
    packet: &[u8],
    address_family: u8,
    write_gate: Option<&Mutex<()>>,
) -> io::Result<()> {
    with_tun_write_gate(write_gate, || {
        raw_write_packet_to_tun(tun_fd, packet, address_family)
    })
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

#[cfg(target_os = "linux")]
fn raw_write_linux_vnet_frame_to_tun_gated(
    tun_fd: &BorrowedTunFd,
    frame: &[u8],
    write_gate: Option<&Mutex<()>>,
) -> io::Result<()> {
    with_tun_write_gate(write_gate, || raw_write_linux_vnet_frame_to_tun(tun_fd, frame))
}

#[cfg(target_os = "linux")]
fn raw_write_linux_vnet_vectored_frame_to_tun<P: LinuxVnetPacketBatch + ?Sized>(
    tun_fd: &BorrowedTunFd,
    packets: &P,
    frame: &LinuxVnetWriteFrame,
    iov: &mut Vec<libc::iovec>,
    write_gate: Option<&Mutex<()>>,
) -> io::Result<usize> {
    let first_packet = packets.packet_slice(frame.first_packet_index);
    let first_payload = &first_packet[frame.first_payload_offset..];
    let first_header = frame.first_header.as_slice();
    let iov_count = frame
        .payload_segments
        .len()
        .saturating_add(2)
        .saturating_add(usize::from(!first_header.is_empty()));
    if iov_count > LINUX_IOV_MAX {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Linux vnet writev iovec count exceeds IOV_MAX",
        ));
    }

    let mut expected = LINUX_VIRTIO_NET_HDR_LEN
        .saturating_add(first_header.len())
        .saturating_add(first_payload.len());
    iov.clear();
    if iov.capacity() < iov_count {
        iov.reserve(iov_count - iov.capacity());
    }
    iov.push(libc::iovec {
        iov_base: frame.virtio_header.as_ptr() as *mut libc::c_void,
        iov_len: frame.virtio_header.len(),
    });
    if !first_header.is_empty() {
        iov.push(libc::iovec {
            iov_base: first_header.as_ptr() as *mut libc::c_void,
            iov_len: first_header.len(),
        });
    }
    iov.push(libc::iovec {
        iov_base: first_payload.as_ptr() as *mut libc::c_void,
        iov_len: first_payload.len(),
    });
    let mut borrowed_segment_bytes = first_header
        .len()
        .saturating_add(first_payload.len());
    for segment in &frame.payload_segments {
        let payload = &packets.packet_slice(segment.packet_index)[segment.payload_offset..];
        expected = expected.saturating_add(payload.len());
        borrowed_segment_bytes = borrowed_segment_bytes.saturating_add(payload.len());
        iov.push(libc::iovec {
            iov_base: payload.as_ptr() as *mut libc::c_void,
            iov_len: payload.len(),
        });
    }
    let written = with_tun_write_gate(write_gate, || unsafe {
        libc::writev(
            tun_fd.as_raw_fd(),
            iov.as_ptr(),
            iov.len() as libc::c_int,
        )
    });
    let result = raw_tun_write_result(written, expected);
    iov.clear();
    result?;
    if frame.payload_segments.is_empty() {
        return Ok(expected);
    }
    crate::pipeline_profile::record_tun_write_vnet_gro_vectored_frame(
        frame.payload_segments.len().saturating_add(1),
        borrowed_segment_bytes,
    );
    Ok(expected)
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn fips_unix_packet_debug_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("NVPN_FIPS_PACKET_DEBUG")
            .ok()
            .is_some_and(|value| fips_packet_debug_value_enabled(&value))
    })
}

fn fips_packet_debug_value_enabled(value: &str) -> bool {
    let value = value.trim();
    !(value.is_empty()
        || value == "0"
        || value.eq_ignore_ascii_case("false")
        || value.eq_ignore_ascii_case("no")
        || value.eq_ignore_ascii_case("off"))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn describe_ip_packet(packet: &[u8]) -> String {
    match packet.first().map(|byte| byte >> 4) {
        Some(4) if packet.len() >= 20 => {
            let src = std::net::Ipv4Addr::new(packet[12], packet[13], packet[14], packet[15]);
            let dst = std::net::Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]);
            format!("IPv4 proto={} {src}->{dst}", packet[9])
        }
        Some(6) if packet.len() >= 40 => {
            let src = std::net::Ipv6Addr::from([
                packet[8], packet[9], packet[10], packet[11], packet[12], packet[13], packet[14],
                packet[15], packet[16], packet[17], packet[18], packet[19], packet[20],
                packet[21], packet[22], packet[23],
            ]);
            let dst = std::net::Ipv6Addr::from([
                packet[24], packet[25], packet[26], packet[27], packet[28], packet[29],
                packet[30], packet[31], packet[32], packet[33], packet[34], packet[35],
                packet[36], packet[37], packet[38], packet[39],
            ]);
            format!("IPv6 next_header={} {src}->{dst}", packet[6])
        }
        Some(version) => format!("IP version {version} short packet"),
        None => "empty packet".to_string(),
    }
}

#[cfg(target_os = "windows")]
include!("windows_runtime_type.rs");
