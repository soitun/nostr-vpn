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
