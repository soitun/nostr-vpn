const ANDROID_TUN_PACKET_HEADROOM: usize = 128;
const ANDROID_TUN_PACKET_MAX: usize = 65_535;
const ANDROID_TUN_POLL_TIMEOUT_MS: i32 = 100;

struct AndroidTunRuntime {
    fd: c_int,
    stop: Arc<AtomicBool>,
    read_thread: Option<std::thread::JoinHandle<()>>,
    write_thread: Option<std::thread::JoinHandle<()>>,
}

impl AndroidTunRuntime {
    fn start(
        fd: c_int,
        outbound_tx: tokio_mpsc::Sender<Vec<u8>>,
        inbound_rx: Arc<Mutex<mpsc::Receiver<Vec<u8>>>>,
        packet_capacity: usize,
    ) -> Result<Self> {
        if let Err(error) = set_android_tun_nonblocking(fd) {
            close_android_tun_fd(fd);
            return Err(error);
        }

        let stop = Arc::new(AtomicBool::new(false));
        let read_stop = Arc::clone(&stop);
        let read_thread = match std::thread::Builder::new()
            .name("nvpn-tun-read".to_string())
            .spawn(move || android_tun_read_loop(fd, outbound_tx, read_stop, packet_capacity))
        {
            Ok(thread) => thread,
            Err(error) => {
                close_android_tun_fd(fd);
                return Err(anyhow!("failed to spawn Android tun read thread: {error}"));
            }
        };

        let write_stop = Arc::clone(&stop);
        let write_thread = match std::thread::Builder::new()
            .name("nvpn-tun-write".to_string())
            .spawn(move || android_tun_write_loop(fd, inbound_rx, write_stop))
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
                return Err(anyhow!("failed to spawn Android tun write thread: {error}"));
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
            close_android_tun_fd(self.fd);
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

impl Drop for AndroidTunRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn android_tun_packet_capacity(mtu: u16) -> usize {
    usize::from(mtu)
        .saturating_add(ANDROID_TUN_PACKET_HEADROOM)
        .clamp(1280, ANDROID_TUN_PACKET_MAX)
}

fn android_tun_read_loop(
    fd: c_int,
    outbound_tx: tokio_mpsc::Sender<Vec<u8>>,
    stop: Arc<AtomicBool>,
    packet_capacity: usize,
) {
    while wait_android_tun_fd(fd, libc::POLLIN, &stop) {
        match read_android_tun_packet(fd, packet_capacity) {
            AndroidTunRead::Packet(packet) => {
                if !send_android_tun_packet(&outbound_tx, &stop, packet) {
                    break;
                }
            }
            AndroidTunRead::WouldBlock => {}
            AndroidTunRead::Stopped => break,
        }
    }
    stop.store(true, Ordering::Relaxed);
}

fn send_android_tun_packet(
    outbound_tx: &tokio_mpsc::Sender<Vec<u8>>,
    stop: &AtomicBool,
    packet: Vec<u8>,
) -> bool {
    if stop.load(Ordering::Relaxed) {
        return false;
    }
    outbound_tx.blocking_send(packet).is_ok()
}

fn android_tun_write_loop(
    fd: c_int,
    inbound_rx: Arc<Mutex<mpsc::Receiver<Vec<u8>>>>,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Relaxed) {
        let packet = {
            let rx = match inbound_rx.lock() {
                Ok(rx) => rx,
                Err(_) => break,
            };
            match rx.recv_timeout(Duration::from_millis(
                u64::try_from(ANDROID_TUN_POLL_TIMEOUT_MS).unwrap_or(100),
            )) {
                Ok(packet) => packet,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        };
        if !write_android_tun_packet(fd, &packet, &stop) {
            break;
        }
    }
    stop.store(true, Ordering::Relaxed);
}

enum AndroidTunRead {
    Packet(Vec<u8>),
    WouldBlock,
    Stopped,
}

fn read_android_tun_packet(fd: c_int, packet_capacity: usize) -> AndroidTunRead {
    let mut packet = Vec::<u8>::with_capacity(packet_capacity);
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
            unsafe {
                packet.set_len(len);
            }
            return AndroidTunRead::Packet(packet);
        }
        if read == 0 {
            return AndroidTunRead::Stopped;
        }
        let error = std::io::Error::last_os_error();
        if android_errno_is_interrupted(error.raw_os_error()) {
            continue;
        }
        if android_errno_is_again(error.raw_os_error()) {
            return AndroidTunRead::WouldBlock;
        }
        return AndroidTunRead::Stopped;
    }
}

fn write_android_tun_packet(fd: c_int, packet: &[u8], stop: &AtomicBool) -> bool {
    if packet.is_empty() {
        return true;
    }
    while wait_android_tun_fd(fd, libc::POLLOUT, stop) {
        let written =
            unsafe { libc::write(fd, packet.as_ptr().cast::<libc::c_void>(), packet.len()) };
        if written == isize::try_from(packet.len()).unwrap_or(-1) {
            return true;
        }
        if written >= 0 {
            return false;
        }
        let error = std::io::Error::last_os_error();
        if android_errno_is_interrupted(error.raw_os_error())
            || android_errno_is_again(error.raw_os_error())
        {
            continue;
        }
        return false;
    }
    false
}

fn wait_android_tun_fd(fd: c_int, events: i16, stop: &AtomicBool) -> bool {
    while !stop.load(Ordering::Relaxed) {
        let mut poll_fd = libc::pollfd {
            fd,
            events,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut poll_fd, 1, ANDROID_TUN_POLL_TIMEOUT_MS) };
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
        if android_errno_is_interrupted(error.raw_os_error()) {
            continue;
        }
        return false;
    }
    false
}

fn set_android_tun_nonblocking(fd: c_int) -> Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(anyhow!(
            "failed to read Android tun fd flags: {}",
            std::io::Error::last_os_error()
        ));
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(anyhow!(
            "failed to set Android tun fd nonblocking: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn close_android_tun_fd(fd: c_int) {
    if fd >= 0 {
        unsafe {
            libc::close(fd);
        }
    }
}

fn android_errno_is_interrupted(error: Option<i32>) -> bool {
    error == Some(libc::EINTR)
}

fn android_errno_is_again(error: Option<i32>) -> bool {
    error == Some(libc::EAGAIN) || error == Some(libc::EWOULDBLOCK)
}
