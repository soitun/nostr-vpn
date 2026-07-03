const NATIVE_TUN_PACKET_HEADROOM: usize = 128;
const NATIVE_TUN_PACKET_MAX: usize = 65_535;
const NATIVE_TUN_POLL_TIMEOUT_MS: i32 = 100;
#[cfg(target_os = "ios")]
const IOS_UTUN_HEADER_LEN: usize = 4;

struct NativeTunRuntime {
    fd: c_int,
    stop: Arc<AtomicBool>,
    read_thread: Option<std::thread::JoinHandle<()>>,
    write_thread: Option<std::thread::JoinHandle<()>>,
}

impl NativeTunRuntime {
    fn start(
        fd: c_int,
        outbound_tx: tokio_mpsc::Sender<Vec<u8>>,
        inbound_rx: mpsc::Receiver<Vec<u8>>,
        packet_capacity: usize,
        counters: Arc<MobileTunAtomicCounters>,
    ) -> Result<Self> {
        let fd = prepare_mobile_tun_fd(fd)?;

        let stop = Arc::new(AtomicBool::new(false));
        let read_stop = Arc::clone(&stop);
        let read_counters = Arc::clone(&counters);
        let read_thread = match std::thread::Builder::new()
            .name("nvpn-tun-read".to_string())
            .spawn(move || {
                native_tun_read_loop(fd, outbound_tx, read_stop, read_counters, packet_capacity)
            })
        {
            Ok(thread) => thread,
            Err(error) => {
                close_mobile_tun_fd(fd);
                return Err(anyhow!("failed to spawn native tun read thread: {error}"));
            }
        };

        let write_stop = Arc::clone(&stop);
        let write_counters = Arc::clone(&counters);
        let write_thread = match std::thread::Builder::new()
            .name("nvpn-tun-write".to_string())
            .spawn(move || native_tun_write_loop(fd, inbound_rx, write_stop, write_counters))
        {
            Ok(thread) => thread,
            Err(error) => {
                let mut runtime = Self {
                    fd,
                    stop,
                    read_thread: Some(read_thread),
                    write_thread: None,
                };
                runtime.shutdown();
                return Err(anyhow!("failed to spawn native tun write thread: {error}"));
            }
        };

        Ok(Self {
            fd,
            stop,
            read_thread: Some(read_thread),
            write_thread: Some(write_thread),
        })
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if self.fd >= 0 {
            close_mobile_tun_fd(self.fd);
            self.fd = -1;
        }
    }

    fn join(&mut self) {
        if let Some(thread) = self.read_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.write_thread.take() {
            let _ = thread.join();
        }
    }

    fn shutdown(&mut self) {
        self.stop();
        self.join();
    }
}

impl Drop for NativeTunRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn native_tun_packet_capacity(mtu: u16) -> usize {
    usize::from(mtu)
        .saturating_add(NATIVE_TUN_PACKET_HEADROOM)
        .clamp(1280, NATIVE_TUN_PACKET_MAX)
}

fn native_tun_read_loop(
    fd: c_int,
    outbound_tx: tokio_mpsc::Sender<Vec<u8>>,
    stop: Arc<AtomicBool>,
    counters: Arc<MobileTunAtomicCounters>,
    packet_capacity: usize,
) {
    while wait_mobile_tun_fd(fd, libc::POLLIN, &stop) {
        for _ in 0..MOBILE_FIPS_SEND_BATCH {
            match read_mobile_tun_packet(fd, packet_capacity) {
                NativeTunRead::Packet(packet) => {
                    counters.note_read(packet.len());
                    if !send_native_tun_packet(&outbound_tx, &stop, packet) {
                        counters.note_drop();
                        stop.store(true, Ordering::Relaxed);
                        return;
                    }
                }
                NativeTunRead::WouldBlock => break,
                NativeTunRead::Stopped => {
                    stop.store(true, Ordering::Relaxed);
                    return;
                }
            }
        }
    }
    stop.store(true, Ordering::Relaxed);
}

fn send_native_tun_packet(
    outbound_tx: &tokio_mpsc::Sender<Vec<u8>>,
    stop: &AtomicBool,
    packet: Vec<u8>,
) -> bool {
    if stop.load(Ordering::Relaxed) {
        return false;
    }
    outbound_tx.blocking_send(packet).is_ok()
}

fn native_tun_write_loop(
    fd: c_int,
    inbound_rx: mpsc::Receiver<Vec<u8>>,
    stop: Arc<AtomicBool>,
    counters: Arc<MobileTunAtomicCounters>,
) {
    while !stop.load(Ordering::Relaxed) {
        let packet = match inbound_rx.recv() {
            Ok(packet) => packet,
            Err(_) => break,
        };
        let packet_len = packet.len();
        if !write_mobile_tun_packet(fd, &packet, &stop) {
            counters.note_drop();
            break;
        }
        counters.note_write(packet_len);
        for _ in 1..MOBILE_FIPS_RECV_BATCH {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let packet = match inbound_rx.try_recv() {
                Ok(packet) => packet,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    stop.store(true, Ordering::Relaxed);
                    return;
                }
            };
            let packet_len = packet.len();
            if !write_mobile_tun_packet(fd, &packet, &stop) {
                counters.note_drop();
                stop.store(true, Ordering::Relaxed);
                return;
            }
            counters.note_write(packet_len);
        }
    }
    stop.store(true, Ordering::Relaxed);
}

enum NativeTunRead {
    Packet(Vec<u8>),
    WouldBlock,
    Stopped,
}

fn read_mobile_tun_packet(fd: c_int, packet_capacity: usize) -> NativeTunRead {
    let mut packet = Vec::<u8>::with_capacity(native_tun_read_capacity(packet_capacity));
    loop {
        let read = unsafe {
            libc::read(
                fd,
                packet.as_mut_ptr().cast::<libc::c_void>(),
                packet.capacity(),
            )
        };
        if read > 0 {
            let len = usize::try_from(read).unwrap_or(0);
            if !native_tun_finish_read(&mut packet, len) {
                return NativeTunRead::Stopped;
            }
            return NativeTunRead::Packet(packet);
        }
        if read == 0 {
            return NativeTunRead::Stopped;
        }
        let error = std::io::Error::last_os_error();
        if mobile_tun_errno_is_interrupted(error.raw_os_error()) {
            continue;
        }
        if mobile_tun_errno_is_again(error.raw_os_error()) {
            return NativeTunRead::WouldBlock;
        }
        return NativeTunRead::Stopped;
    }
}

#[cfg(target_os = "android")]
fn native_tun_read_capacity(packet_capacity: usize) -> usize {
    packet_capacity
}

#[cfg(target_os = "ios")]
fn native_tun_read_capacity(packet_capacity: usize) -> usize {
    packet_capacity.saturating_add(IOS_UTUN_HEADER_LEN)
}

#[cfg(target_os = "android")]
fn native_tun_finish_read(packet: &mut Vec<u8>, len: usize) -> bool {
    unsafe {
        packet.set_len(len);
    }
    true
}

#[cfg(target_os = "ios")]
fn native_tun_finish_read(packet: &mut Vec<u8>, len: usize) -> bool {
    if len <= IOS_UTUN_HEADER_LEN {
        return false;
    }
    unsafe {
        std::ptr::copy(
            packet.as_ptr().add(IOS_UTUN_HEADER_LEN),
            packet.as_mut_ptr(),
            len - IOS_UTUN_HEADER_LEN,
        );
        packet.set_len(len - IOS_UTUN_HEADER_LEN);
    }
    true
}

#[cfg(target_os = "android")]
fn write_mobile_tun_packet(fd: c_int, packet: &[u8], stop: &AtomicBool) -> bool {
    if packet.is_empty() {
        return true;
    }
    while wait_mobile_tun_fd(fd, libc::POLLOUT, stop) {
        let written =
            unsafe { libc::write(fd, packet.as_ptr().cast::<libc::c_void>(), packet.len()) };
        if written == isize::try_from(packet.len()).unwrap_or(-1) {
            return true;
        }
        if written >= 0 {
            return false;
        }
        let error = std::io::Error::last_os_error();
        if mobile_tun_errno_is_interrupted(error.raw_os_error())
            || mobile_tun_errno_is_again(error.raw_os_error())
        {
            continue;
        }
        return false;
    }
    false
}

#[cfg(target_os = "ios")]
fn write_mobile_tun_packet(fd: c_int, packet: &[u8], stop: &AtomicBool) -> bool {
    if packet.is_empty() {
        return true;
    }
    let Some(header) = ios_utun_packet_header(packet) else {
        return true;
    };
    while wait_mobile_tun_fd(fd, libc::POLLOUT, stop) {
        let mut iov = [
            libc::iovec {
                iov_base: header.as_ptr().cast::<libc::c_void>().cast_mut(),
                iov_len: header.len(),
            },
            libc::iovec {
                iov_base: packet.as_ptr().cast::<libc::c_void>().cast_mut(),
                iov_len: packet.len(),
            },
        ];
        let written = unsafe { libc::writev(fd, iov.as_mut_ptr(), iov.len() as c_int) };
        if written == isize::try_from(packet.len() + IOS_UTUN_HEADER_LEN).unwrap_or(-1) {
            return true;
        }
        if written >= 0 {
            return false;
        }
        let error = std::io::Error::last_os_error();
        if mobile_tun_errno_is_interrupted(error.raw_os_error())
            || mobile_tun_errno_is_again(error.raw_os_error())
        {
            continue;
        }
        return false;
    }
    false
}

#[cfg(target_os = "ios")]
fn ios_utun_packet_header(packet: &[u8]) -> Option<[u8; IOS_UTUN_HEADER_LEN]> {
    let family = match packet.first().map(|byte| byte >> 4) {
        Some(4) => libc::AF_INET,
        Some(6) => libc::AF_INET6,
        _ => return None,
    };
    Some([0, 0, 0, u8::try_from(family).ok()?])
}

fn wait_mobile_tun_fd(fd: c_int, events: i16, stop: &AtomicBool) -> bool {
    while !stop.load(Ordering::Relaxed) {
        let mut poll_fd = libc::pollfd {
            fd,
            events,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut poll_fd, 1, NATIVE_TUN_POLL_TIMEOUT_MS) };
        if ready > 0 {
            if poll_fd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                return false;
            }
            return poll_fd.revents & events != 0;
        }
        if ready == 0 {
            continue;
        }
        let error = std::io::Error::last_os_error();
        if mobile_tun_errno_is_interrupted(error.raw_os_error()) {
            continue;
        }
        return false;
    }
    false
}

fn prepare_mobile_tun_fd(fd: c_int) -> Result<c_int> {
    let fd = native_tun_owned_fd(fd)?;
    if let Err(error) = set_mobile_tun_nonblocking(fd) {
        close_mobile_tun_fd(fd);
        return Err(error);
    }
    Ok(fd)
}

#[cfg(target_os = "android")]
fn native_tun_owned_fd(fd: c_int) -> Result<c_int> {
    Ok(fd)
}

#[cfg(target_os = "ios")]
fn native_tun_owned_fd(fd: c_int) -> Result<c_int> {
    let duplicate = unsafe { libc::dup(fd) };
    if duplicate < 0 {
        return Err(anyhow!(
            "failed to duplicate iOS utun fd: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(duplicate)
}

#[cfg(target_os = "ios")]
fn current_ios_utun_fd() -> Result<c_int> {
    let mut info = unsafe { std::mem::zeroed::<libc::ctl_info>() };
    let name = b"com.apple.net.utun_control\0";
    unsafe {
        std::ptr::copy_nonoverlapping(
            name.as_ptr().cast::<libc::c_char>(),
            info.ctl_name.as_mut_ptr(),
            name.len().min(info.ctl_name.len()),
        );
    }
    for fd in 0..=1024 {
        let mut addr = unsafe { std::mem::zeroed::<libc::sockaddr_ctl>() };
        let mut len = std::mem::size_of::<libc::sockaddr_ctl>() as libc::socklen_t;
        let result = unsafe {
            libc::getpeername(
                fd,
                (&mut addr as *mut libc::sockaddr_ctl).cast::<libc::sockaddr>(),
                &mut len,
            )
        };
        if result != 0 || i32::from(addr.sc_family) != libc::AF_SYSTEM {
            continue;
        }
        if info.ctl_id == 0 && unsafe { libc::ioctl(fd, libc::CTLIOCGINFO, &mut info) } != 0 {
            continue;
        }
        if addr.sc_id == info.ctl_id {
            return Ok(fd);
        }
    }
    Err(anyhow!("failed to locate iOS utun fd"))
}

fn set_mobile_tun_nonblocking(fd: c_int) -> Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(anyhow!(
            "failed to read native tun fd flags: {}",
            std::io::Error::last_os_error()
        ));
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(anyhow!(
            "failed to set native tun fd nonblocking: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn close_mobile_tun_fd(fd: c_int) {
    if fd >= 0 {
        unsafe {
            libc::close(fd);
        }
    }
}

#[cfg(target_os = "android")]
fn reject_unattached_mobile_tun_fd(fd: c_int) {
    close_mobile_tun_fd(fd);
}

#[cfg(target_os = "ios")]
fn reject_unattached_mobile_tun_fd(_fd: c_int) {}

fn mobile_tun_errno_is_interrupted(error: Option<i32>) -> bool {
    error == Some(libc::EINTR)
}

fn mobile_tun_errno_is_again(error: Option<i32>) -> bool {
    error == Some(libc::EAGAIN) || error == Some(libc::EWOULDBLOCK)
}
