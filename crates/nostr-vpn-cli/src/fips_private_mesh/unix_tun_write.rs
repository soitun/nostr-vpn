#[cfg(any(target_os = "linux", target_os = "macos"))]
fn flush_direct_endpoint_packet_batch_to_tun_blocking(
    tun_fd: BorrowedTunFd,
    packet_batch: &mut DirectTunWriteBatch,
    stop: &AtomicBool,
    #[cfg(target_os = "linux")] vnet_write_preparer: &mut LinuxVnetWritePreparer,
) {
    if packet_batch.is_empty() {
        return;
    }
    let _t = crate::pipeline_profile::Timer::start(crate::pipeline_profile::Stage::TunWriteBatch);

    #[cfg(target_os = "linux")]
    {
        let packet_count = packet_batch.len();
        let packet_bytes = packet_batch.bytes();
        write_linux_vnet_packet_batch_to_tun_blocking(
            tun_fd,
            packet_batch,
            packet_count,
            packet_bytes,
            stop,
            vnet_write_preparer,
        );
        packet_batch.clear();
        return;
    }

    #[cfg(target_os = "macos")]
    {
        for packet in packet_batch.run_slices() {
            write_packet_to_tun_blocking(tun_fd, packet, stop);
        }
        packet_batch.clear();
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn write_packet_to_tun_blocking(
    fd: BorrowedTunFd,
    packet: &[u8],
    stop: &AtomicBool,
) {
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
fn write_linux_vnet_packet_batch_to_tun_blocking<P: LinuxVnetPacketBatch + ?Sized>(
    tun_fd: BorrowedTunFd,
    packets: &P,
    packet_count: usize,
    packet_bytes: usize,
    stop: &AtomicBool,
    preparer: &mut LinuxVnetWritePreparer,
) {
    preparer.prepare(packets);
    crate::pipeline_profile::record_tun_write_packets(packet_count, packet_bytes);
    let frame_count = preparer.frames().len();
    for frame_index in 0..frame_count {
        let frame = preparer.frames()[frame_index];
        write_linux_vnet_prepared_frame_to_tun_blocking(tun_fd, preparer, frame, stop);
    }

    // packet_refs borrow `packets`; keep them scoped to this synchronous write pass.
    preparer.packet_refs.clear();
}

#[cfg(target_os = "linux")]
fn write_linux_vnet_prepared_frame_to_tun_blocking(
    tun_fd: BorrowedTunFd,
    preparer: &mut LinuxVnetWritePreparer,
    frame: LinuxVnetPreparedWriteFrame,
    stop: &AtomicBool,
) {
    match frame {
        LinuxVnetPreparedWriteFrame::RawPacket(packet_index) => {
            preparer.packet_ref(packet_index).with_slice(|packet| {
                write_linux_vnet_raw_packet_to_tun_blocking(tun_fd, packet, stop)
            })
        }
        LinuxVnetPreparedWriteFrame::Vectored(frame_index) => {
            write_linux_vnet_vectored_frame_to_tun_blocking(
                tun_fd,
                preparer,
                frame_index,
                stop,
            )
        }
    }
}

#[cfg(target_os = "linux")]
fn write_linux_vnet_raw_packet_to_tun_blocking(
    tun_fd: BorrowedTunFd,
    packet: &[u8],
    stop: &AtomicBool,
) {
    let _t = crate::pipeline_profile::Timer::start(crate::pipeline_profile::Stage::TunWrite);
    loop {
        if stop.load(Ordering::Acquire) {
            return;
        }
        match raw_write_packet_to_tun(&tun_fd, packet, 0) {
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
fn write_linux_vnet_vectored_frame_to_tun_blocking(
    tun_fd: BorrowedTunFd,
    preparer: &mut LinuxVnetWritePreparer,
    frame_index: usize,
    stop: &AtomicBool,
) {
    let _t = crate::pipeline_profile::Timer::start(crate::pipeline_profile::Stage::TunWrite);
    loop {
        if stop.load(Ordering::Acquire) {
            return;
        }
        match preparer.write_vectored_frame_to_tun(&tun_fd, frame_index) {
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
}

#[cfg(target_os = "linux")]
fn raw_write_linux_vnet_vectored_frame_to_tun(
    tun_fd: &BorrowedTunFd,
    packet_refs: &[LinuxVnetPacketRef],
    frame: &LinuxVnetWriteFrame,
    iov: &mut Vec<libc::iovec>,
) -> io::Result<usize> {
    let first_ref = packet_refs[frame.first_packet_index];
    let first_header = frame.first_header.as_slice();
    let first_payload_len = first_ref.len_from_offset(frame.first_payload_offset);
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
        .saturating_add(first_payload_len);
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
    iov.push(first_ref.iovec_from_offset(frame.first_payload_offset));
    let mut borrowed_segment_bytes = first_header.len().saturating_add(first_payload_len);
    for segment in &frame.payload_segments {
        let packet_ref = packet_refs[segment.packet_index];
        let payload_len = packet_ref.len_from_offset(segment.payload_offset);
        expected = expected.saturating_add(payload_len);
        borrowed_segment_bytes = borrowed_segment_bytes.saturating_add(payload_len);
        iov.push(packet_ref.iovec_from_offset(segment.payload_offset));
    }
    let written = unsafe {
        libc::writev(
            tun_fd.as_raw_fd(),
            iov.as_ptr(),
            iov.len() as libc::c_int,
        )
    };
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
