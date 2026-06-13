const LINUX_VIRTIO_NET_HDR_LEN: usize = 10;

const LINUX_VIRTIO_NET_HDR_F_NEEDS_CSUM: u8 = 0x01;
const LINUX_VIRTIO_NET_HDR_GSO_NONE: u8 = 0;
const LINUX_VIRTIO_NET_HDR_GSO_TCPV4: u8 = 1;
const LINUX_VIRTIO_NET_HDR_GSO_TCPV6: u8 = 4;
const LINUX_VIRTIO_NET_HDR_GSO_UDP_L4: u8 = 5;
const LINUX_VIRTIO_NET_HDR_GSO_ECN: u8 = 0x80;

const LINUX_TCP_FLAGS_OFFSET: usize = 13;
const LINUX_TCP_FLAG_FIN: u8 = 0x01;
const LINUX_TCP_FLAG_PSH: u8 = 0x08;
const LINUX_TCP_FLAG_ACK: u8 = 0x10;
const LINUX_IPPROTO_TCP: u8 = 6;
const LINUX_IPPROTO_UDP: u8 = 17;
const LINUX_IPV4_SRC_ADDR_OFFSET: usize = 12;
const LINUX_IPV6_SRC_ADDR_OFFSET: usize = 8;

#[repr(C)]
union LinuxIfReqIfru {
    ifru_flags: libc::c_short,
    ifru_mtu: libc::c_int,
}

#[repr(C)]
struct LinuxIfReq {
    ifr_name: [libc::c_uchar; libc::IFNAMSIZ],
    ifr_ifru: LinuxIfReqIfru,
}

struct LinuxVnetTun {
    fd: RawFd,
    name: String,
    _udp_gso: bool,
}

impl Drop for LinuxVnetTun {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.fd);
        }
    }
}

impl AsRawFd for LinuxVnetTun {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}

impl LinuxVnetTun {
    fn new(name: &str) -> Result<Self> {
        if name.parse::<i32>().is_ok() {
            return Err(anyhow!(
                "NVPN_FIPS_LINUX_TUN_VNET=1 cannot adopt a pre-opened fd; pass an interface name"
            ));
        }
        if name.len() >= libc::IFNAMSIZ {
            return Err(anyhow!("invalid Linux TUN interface name '{name}'"));
        }

        let fd = unsafe {
            libc::open(
                b"/dev/net/tun\0".as_ptr().cast(),
                libc::O_RDWR | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error()).context("open /dev/net/tun");
        }

        let result = Self::configure_fd(fd, name);
        if result.is_err() {
            unsafe {
                libc::close(fd);
            }
        }
        result
    }

    fn configure_fd(fd: RawFd, name: &str) -> Result<Self> {
        let mut ifr = LinuxIfReq {
            ifr_name: [0; libc::IFNAMSIZ],
            ifr_ifru: LinuxIfReqIfru {
                ifru_flags: (libc::IFF_TUN | libc::IFF_NO_PI | libc::IFF_VNET_HDR)
                    as libc::c_short,
            },
        };
        let name_bytes = name.as_bytes();
        ifr.ifr_name[..name_bytes.len()].copy_from_slice(name_bytes);

        let rc = unsafe { libc::ioctl(fd, libc::TUNSETIFF as _, &ifr) };
        if rc < 0 {
            return Err(io::Error::last_os_error()).context("TUNSETIFF IFF_VNET_HDR");
        }

        let tcp_offloads = libc::TUN_F_CSUM | libc::TUN_F_TSO4 | libc::TUN_F_TSO6;
        let rc = unsafe { libc::ioctl(fd, libc::TUNSETOFFLOAD as _, tcp_offloads) };
        if rc < 0 {
            return Err(io::Error::last_os_error()).context("TUNSETOFFLOAD TCP offloads");
        }

        let udp_offloads = tcp_offloads | libc::TUN_F_USO4 | libc::TUN_F_USO6;
        let udp_gso = unsafe { libc::ioctl(fd, libc::TUNSETOFFLOAD as _, udp_offloads) } >= 0;

        eprintln!("fips: Linux vnet TUN enabled on {name}; udp_gso={udp_gso}");
        Ok(Self {
            fd,
            name: name.to_string(),
            _udp_gso: udp_gso,
        })
    }

    fn set_non_blocking(self) -> Result<Self> {
        let flags = unsafe { libc::fcntl(self.fd, libc::F_GETFL) };
        if flags < 0 {
            return Err(io::Error::last_os_error()).context("F_GETFL Linux vnet TUN");
        }
        let rc = unsafe { libc::fcntl(self.fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
        if rc < 0 {
            return Err(io::Error::last_os_error()).context("F_SETFL O_NONBLOCK Linux vnet TUN");
        }
        Ok(self)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn read_packets_into(
        &self,
        scratch: &mut [u8],
        batch: &mut TunPipelineBatch,
    ) -> io::Result<usize> {
        let read_len = unsafe {
            libc::read(
                self.fd,
                scratch.as_mut_ptr().cast::<libc::c_void>(),
                scratch.len(),
            )
        };
        if read_len < 0 {
            return Err(io::Error::last_os_error());
        }
        if read_len == 0 {
            return Ok(0);
        }
        handle_linux_vnet_read(&mut scratch[..read_len as usize], batch)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LinuxVirtioNetHdr {
    flags: u8,
    gso_type: u8,
    hdr_len: u16,
    gso_size: u16,
    csum_start: u16,
    csum_offset: u16,
}

impl LinuxVirtioNetHdr {
    fn decode(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() < LINUX_VIRTIO_NET_HDR_LEN {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "short virtio net header",
            ));
        }
        Ok(Self {
            flags: bytes[0],
            gso_type: bytes[1],
            hdr_len: u16::from_ne_bytes([bytes[2], bytes[3]]),
            gso_size: u16::from_ne_bytes([bytes[4], bytes[5]]),
            csum_start: u16::from_ne_bytes([bytes[6], bytes[7]]),
            csum_offset: u16::from_ne_bytes([bytes[8], bytes[9]]),
        })
    }

    fn encode(self, bytes: &mut [u8]) {
        bytes[0] = self.flags;
        bytes[1] = self.gso_type;
        bytes[2..4].copy_from_slice(&self.hdr_len.to_ne_bytes());
        bytes[4..6].copy_from_slice(&self.gso_size.to_ne_bytes());
        bytes[6..8].copy_from_slice(&self.csum_start.to_ne_bytes());
        bytes[8..10].copy_from_slice(&self.csum_offset.to_ne_bytes());
    }
}

struct LinuxVnetWriteFrame {
    bytes: Vec<u8>,
    tcp4_gro: Option<LinuxVnetTcp4GroState>,
}

#[derive(Clone, Debug)]
struct LinuxVnetTcp4GroState {
    ip_header_len: usize,
    tcp_header_len: usize,
    gso_size: usize,
    payload_len: usize,
    next_seq: u32,
    psh_set: bool,
    flow: LinuxVnetTcp4GroFlow,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LinuxVnetTcp4GroFlow {
    tos: u8,
    df_reserved_flags: u8,
    ttl: u8,
    src_addr: [u8; 4],
    dst_addr: [u8; 4],
    src_port: u16,
    dst_port: u16,
    ack: u32,
    tcp_options: Vec<u8>,
}

#[derive(Clone, Debug)]
struct LinuxVnetTcp4GroCandidate {
    ip_header_len: usize,
    tcp_header_len: usize,
    payload_len: usize,
    seq: u32,
    psh_set: bool,
    flow: LinuxVnetTcp4GroFlow,
}

fn linux_vnet_prepare_write_frames(packets: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let mut frames = Vec::with_capacity(packets.len());
    let mut current: Option<LinuxVnetWriteFrame> = None;

    for packet in packets {
        if let Some(frame) = current.as_mut()
            && linux_vnet_try_tcp4_gro_append(frame, packet)
        {
            continue;
        }

        if let Some(frame) = current.take() {
            frames.push(linux_vnet_finish_write_frame(frame));
        }
        current = Some(linux_vnet_start_write_frame(packet));
    }

    if let Some(frame) = current {
        frames.push(linux_vnet_finish_write_frame(frame));
    }

    frames
}

fn linux_vnet_start_write_frame(packet: &[u8]) -> LinuxVnetWriteFrame {
    let mut bytes = Vec::with_capacity(LINUX_VIRTIO_NET_HDR_LEN + packet.len());
    bytes.resize(LINUX_VIRTIO_NET_HDR_LEN, 0);
    bytes.extend_from_slice(packet);
    let tcp4_gro = linux_vnet_tcp4_gro_candidate(packet).map(|candidate| {
        LinuxVnetTcp4GroState {
            ip_header_len: candidate.ip_header_len,
            tcp_header_len: candidate.tcp_header_len,
            gso_size: candidate.payload_len,
            payload_len: candidate.payload_len,
            next_seq: candidate.seq.wrapping_add(candidate.payload_len as u32),
            psh_set: candidate.psh_set,
            flow: candidate.flow,
        }
    });
    LinuxVnetWriteFrame { bytes, tcp4_gro }
}

fn linux_vnet_try_tcp4_gro_append(frame: &mut LinuxVnetWriteFrame, packet: &[u8]) -> bool {
    let Some(state) = frame.tcp4_gro.as_mut() else {
        return false;
    };
    if state.psh_set || state.payload_len % state.gso_size != 0 {
        return false;
    }
    let Some(candidate) = linux_vnet_tcp4_gro_candidate(packet) else {
        return false;
    };
    if candidate.flow != state.flow
        || candidate.ip_header_len != state.ip_header_len
        || candidate.tcp_header_len != state.tcp_header_len
        || candidate.seq != state.next_seq
        || candidate.payload_len > state.gso_size
    {
        return false;
    }

    let header_len = candidate.ip_header_len + candidate.tcp_header_len;
    let packet_start = LINUX_VIRTIO_NET_HDR_LEN;
    let coalesced_packet_len = frame.bytes.len() - packet_start + packet.len() - header_len;
    if coalesced_packet_len > u16::MAX as usize {
        return false;
    }

    frame.bytes.extend_from_slice(&packet[header_len..]);
    state.payload_len += candidate.payload_len;
    state.next_seq = state.next_seq.wrapping_add(candidate.payload_len as u32);
    if candidate.psh_set {
        let flags_at = packet_start + state.ip_header_len + LINUX_TCP_FLAGS_OFFSET;
        frame.bytes[flags_at] |= LINUX_TCP_FLAG_PSH;
        state.psh_set = true;
    }
    true
}

fn linux_vnet_finish_write_frame(mut frame: LinuxVnetWriteFrame) -> Vec<u8> {
    let Some(state) = frame.tcp4_gro else {
        return frame.bytes;
    };
    if state.payload_len <= state.gso_size {
        return frame.bytes;
    }

    let packet_start = LINUX_VIRTIO_NET_HDR_LEN;
    let packet_len = frame.bytes.len() - packet_start;
    let ip_header_len = state.ip_header_len;
    let tcp_header_len = state.tcp_header_len;
    let transport_len = packet_len - ip_header_len;
    let packet = &mut frame.bytes[packet_start..];

    packet[2..4].copy_from_slice(&(packet_len as u16).to_be_bytes());
    linux_vnet_finalize_ipv4_header_checksum(&mut packet[..ip_header_len]);

    let pseudo = linux_vnet_pseudo_header_sum(
        LINUX_IPPROTO_TCP,
        &packet[LINUX_IPV4_SRC_ADDR_OFFSET..LINUX_IPV4_SRC_ADDR_OFFSET + 4],
        &packet[LINUX_IPV4_SRC_ADDR_OFFSET + 4..LINUX_IPV4_SRC_ADDR_OFFSET + 8],
        transport_len as u16,
    );
    let partial = !linux_vnet_checksum(&[], pseudo);
    let checksum_at = ip_header_len + 16;
    packet[checksum_at..checksum_at + 2].copy_from_slice(&partial.to_be_bytes());

    LinuxVirtioNetHdr {
        flags: LINUX_VIRTIO_NET_HDR_F_NEEDS_CSUM,
        gso_type: LINUX_VIRTIO_NET_HDR_GSO_TCPV4,
        hdr_len: (ip_header_len + tcp_header_len) as u16,
        gso_size: state.gso_size as u16,
        csum_start: ip_header_len as u16,
        csum_offset: 16,
    }
    .encode(&mut frame.bytes[..LINUX_VIRTIO_NET_HDR_LEN]);

    frame.bytes
}

fn linux_vnet_tcp4_gro_candidate(packet: &[u8]) -> Option<LinuxVnetTcp4GroCandidate> {
    if packet.len() < 40 || packet[0] >> 4 != 4 || packet[9] != LINUX_IPPROTO_TCP {
        return None;
    }
    let ip_header_len = usize::from(packet[0] & 0x0f) * 4;
    if !(20..=60).contains(&ip_header_len) || packet.len() < ip_header_len + 20 {
        return None;
    }
    let total_len = usize::from(u16::from_be_bytes([packet[2], packet[3]]));
    if total_len != packet.len() {
        return None;
    }
    let fragment = u16::from_be_bytes([packet[6], packet[7]]);
    if fragment & 0x3fff != 0 {
        return None;
    }

    let tcp_header_len = usize::from(packet[ip_header_len + 12] >> 4) * 4;
    if !(20..=60).contains(&tcp_header_len) || packet.len() < ip_header_len + tcp_header_len {
        return None;
    }
    let flags = packet[ip_header_len + LINUX_TCP_FLAGS_OFFSET];
    let psh_set = match flags {
        LINUX_TCP_FLAG_ACK => false,
        flags if flags == (LINUX_TCP_FLAG_ACK | LINUX_TCP_FLAG_PSH) => true,
        _ => return None,
    };

    let payload_len = packet.len() - ip_header_len - tcp_header_len;
    if payload_len == 0 || payload_len > u16::MAX as usize {
        return None;
    }

    let mut src_addr = [0_u8; 4];
    src_addr.copy_from_slice(&packet[12..16]);
    let mut dst_addr = [0_u8; 4];
    dst_addr.copy_from_slice(&packet[16..20]);
    let tcp = &packet[ip_header_len..];
    let tcp_options = if tcp_header_len > 20 {
        tcp[20..tcp_header_len].to_vec()
    } else {
        Vec::new()
    };

    Some(LinuxVnetTcp4GroCandidate {
        ip_header_len,
        tcp_header_len,
        payload_len,
        seq: u32::from_be_bytes([tcp[4], tcp[5], tcp[6], tcp[7]]),
        psh_set,
        flow: LinuxVnetTcp4GroFlow {
            tos: packet[1],
            df_reserved_flags: packet[6] >> 5,
            ttl: packet[8],
            src_addr,
            dst_addr,
            src_port: u16::from_be_bytes([tcp[0], tcp[1]]),
            dst_port: u16::from_be_bytes([tcp[2], tcp[3]]),
            ack: u32::from_be_bytes([tcp[8], tcp[9], tcp[10], tcp[11]]),
            tcp_options,
        },
    })
}

fn linux_vnet_finalize_ipv4_header_checksum(header: &mut [u8]) {
    header[10] = 0;
    header[11] = 0;
    let checksum = !linux_vnet_checksum(header, 0);
    header[10..12].copy_from_slice(&checksum.to_be_bytes());
}

fn linux_vnet_tun_enabled() -> bool {
    static VALUE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *VALUE.get_or_init(|| linux_vnet_tun_enabled_from_env(std::env::var("NVPN_FIPS_LINUX_TUN_VNET").ok().as_deref()))
}

fn linux_vnet_tun_enabled_from_env(value: Option<&str>) -> bool {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return false;
    };
    !(value == "0"
        || value.eq_ignore_ascii_case("false")
        || value.eq_ignore_ascii_case("no")
        || value.eq_ignore_ascii_case("off"))
}

fn handle_linux_vnet_read(frame: &mut [u8], batch: &mut TunPipelineBatch) -> io::Result<usize> {
    let hdr = LinuxVirtioNetHdr::decode(frame)?;
    let packet = &mut frame[LINUX_VIRTIO_NET_HDR_LEN..];
    let gso_type = hdr.gso_type & !LINUX_VIRTIO_NET_HDR_GSO_ECN;

    if gso_type == LINUX_VIRTIO_NET_HDR_GSO_NONE {
        if hdr.flags & LINUX_VIRTIO_NET_HDR_F_NEEDS_CSUM != 0 {
            linux_vnet_gso_none_checksum(packet, hdr.csum_start, hdr.csum_offset)?;
        }
        push_tun_pipeline_packet(batch, packet);
        return Ok(1);
    }

    if hdr.gso_type & LINUX_VIRTIO_NET_HDR_GSO_ECN != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Linux vnet TUN GSO ECN packets are not supported yet",
        ));
    }
    if hdr.gso_size == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Linux vnet TUN GSO packet has zero segment size",
        ));
    }

    match gso_type {
        LINUX_VIRTIO_NET_HDR_GSO_TCPV4
        | LINUX_VIRTIO_NET_HDR_GSO_TCPV6
        | LINUX_VIRTIO_NET_HDR_GSO_UDP_L4 => {}
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported Linux vnet TUN GSO type {gso_type}"),
            ));
        }
    }

    let Some((&version_byte, _)) = packet.split_first() else {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "empty Linux vnet TUN packet",
        ));
    };
    let is_v6 = match version_byte >> 4 {
        4 => {
            if gso_type != LINUX_VIRTIO_NET_HDR_GSO_TCPV4
                && gso_type != LINUX_VIRTIO_NET_HDR_GSO_UDP_L4
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "IPv4 packet has non-IPv4 GSO type",
                ));
            }
            false
        }
        6 => {
            if gso_type != LINUX_VIRTIO_NET_HDR_GSO_TCPV6
                && gso_type != LINUX_VIRTIO_NET_HDR_GSO_UDP_L4
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "IPv6 packet has non-IPv6 GSO type",
                ));
            }
            true
        }
        version => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid Linux vnet TUN packet IP version {version}"),
            ));
        }
    };

    let mut hdr = hdr;
    if gso_type == LINUX_VIRTIO_NET_HDR_GSO_UDP_L4 {
        hdr.hdr_len = hdr.csum_start.saturating_add(8);
    } else {
        let tcp_data_offset_at = usize::from(hdr.csum_start).saturating_add(12);
        let Some(&data_offset_byte) = packet.get(tcp_data_offset_at) else {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Linux vnet TUN TCP GSO packet is too short",
            ));
        };
        let tcp_header_len = u16::from(data_offset_byte >> 4) * 4;
        if !(20..=60).contains(&tcp_header_len) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid Linux vnet TUN TCP header length {tcp_header_len}"),
            ));
        }
        hdr.hdr_len = hdr.csum_start.saturating_add(tcp_header_len);
    }

    let hdr_len = usize::from(hdr.hdr_len);
    let csum_start = usize::from(hdr.csum_start);
    let csum_at = csum_start.saturating_add(usize::from(hdr.csum_offset));
    if packet.len() < hdr_len || hdr_len < csum_start || csum_at + 1 >= packet.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid Linux vnet TUN GSO header bounds",
        ));
    }

    linux_vnet_gso_split(packet, hdr, gso_type, is_v6, batch)
}

fn linux_vnet_gso_none_checksum(
    packet: &mut [u8],
    csum_start: u16,
    csum_offset: u16,
) -> io::Result<()> {
    let csum_start = usize::from(csum_start);
    let csum_at = csum_start.saturating_add(usize::from(csum_offset));
    if csum_start >= packet.len() || csum_at + 1 >= packet.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid Linux vnet TUN checksum bounds",
        ));
    }
    let initial = u16::from_be_bytes([packet[csum_at], packet[csum_at + 1]]);
    packet[csum_at] = 0;
    packet[csum_at + 1] = 0;
    let checksum = !linux_vnet_checksum(&packet[csum_start..], u64::from(initial));
    packet[csum_at..csum_at + 2].copy_from_slice(&checksum.to_be_bytes());
    Ok(())
}

fn linux_vnet_gso_split(
    packet: &mut [u8],
    hdr: LinuxVirtioNetHdr,
    gso_type: u8,
    is_v6: bool,
    batch: &mut TunPipelineBatch,
) -> io::Result<usize> {
    let ip_header_len = usize::from(hdr.csum_start);
    let transport_csum_at = usize::from(hdr.csum_start + hdr.csum_offset);
    if !is_v6 {
        packet[10] = 0;
        packet[11] = 0;
    }
    packet[transport_csum_at] = 0;
    packet[transport_csum_at + 1] = 0;

    let protocol = if gso_type == LINUX_VIRTIO_NET_HDR_GSO_TCPV4
        || gso_type == LINUX_VIRTIO_NET_HDR_GSO_TCPV6
    {
        LINUX_IPPROTO_TCP
    } else {
        LINUX_IPPROTO_UDP
    };
    let first_tcp_seq = if protocol == LINUX_IPPROTO_TCP {
        let seq_at = usize::from(hdr.csum_start).saturating_add(4);
        u32::from_be_bytes([
            packet[seq_at],
            packet[seq_at + 1],
            packet[seq_at + 2],
            packet[seq_at + 3],
        ])
    } else {
        0
    };

    let src_addr_offset = if is_v6 {
        LINUX_IPV6_SRC_ADDR_OFFSET
    } else {
        LINUX_IPV4_SRC_ADDR_OFFSET
    };
    let addr_len = if is_v6 { 16 } else { 4 };
    if src_addr_offset + addr_len * 2 > packet.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Linux vnet TUN GSO packet is too short for IP addresses",
        ));
    }
    let src = packet[src_addr_offset..src_addr_offset + addr_len].to_vec();
    let dst = packet[src_addr_offset + addr_len..src_addr_offset + addr_len * 2].to_vec();

    let mut next_segment_data_at = usize::from(hdr.hdr_len);
    let mut count = 0_usize;
    while next_segment_data_at < packet.len() {
        let next_segment_end =
            (next_segment_data_at + usize::from(hdr.gso_size)).min(packet.len());
        let segment_data_len = next_segment_end - next_segment_data_at;
        let total_len = usize::from(hdr.hdr_len) + segment_data_len;
        if total_len > u16::MAX as usize {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Linux vnet TUN GSO segment exceeds packet length limit",
            ));
        }

        let mut out = vec![0_u8; total_len];
        out[..ip_header_len].copy_from_slice(&packet[..ip_header_len]);
        if !is_v6 {
            if count > 0 {
                let id = u16::from_be_bytes([out[4], out[5]]).wrapping_add(count as u16);
                out[4..6].copy_from_slice(&id.to_be_bytes());
            }
            out[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
            let ip_checksum = !linux_vnet_checksum(&out[..ip_header_len], 0);
            out[10..12].copy_from_slice(&ip_checksum.to_be_bytes());
        } else {
            out[4..6].copy_from_slice(&((total_len - ip_header_len) as u16).to_be_bytes());
        }

        let csum_start = usize::from(hdr.csum_start);
        let hdr_len = usize::from(hdr.hdr_len);
        out[csum_start..hdr_len].copy_from_slice(&packet[csum_start..hdr_len]);
        if protocol == LINUX_IPPROTO_TCP {
            let tcp_seq = first_tcp_seq.wrapping_add(u32::from(hdr.gso_size) * count as u32);
            out[csum_start + 4..csum_start + 8].copy_from_slice(&tcp_seq.to_be_bytes());
            if next_segment_end != packet.len() {
                out[csum_start + LINUX_TCP_FLAGS_OFFSET] &=
                    !(LINUX_TCP_FLAG_FIN | LINUX_TCP_FLAG_PSH);
            }
        } else {
            let udp_len = (hdr.hdr_len - hdr.csum_start) + segment_data_len as u16;
            out[csum_start + 4..csum_start + 6].copy_from_slice(&udp_len.to_be_bytes());
        }

        out[hdr_len..].copy_from_slice(&packet[next_segment_data_at..next_segment_end]);

        let transport_len = total_len - csum_start;
        let pseudo_sum =
            linux_vnet_pseudo_header_sum(protocol, &src, &dst, transport_len as u16);
        let transport_checksum = !linux_vnet_checksum(&out[csum_start..], pseudo_sum);
        let out_csum_at = csum_start + usize::from(hdr.csum_offset);
        out[out_csum_at..out_csum_at + 2].copy_from_slice(&transport_checksum.to_be_bytes());

        push_tun_pipeline_packet(batch, &out);
        count += 1;
        next_segment_data_at = next_segment_end;
    }

    Ok(count)
}

fn linux_vnet_pseudo_header_sum(protocol: u8, src: &[u8], dst: &[u8], total_len: u16) -> u64 {
    let mut sum = linux_vnet_add_words(0, src);
    sum = linux_vnet_add_words(sum, dst);
    sum += u64::from(protocol);
    sum += u64::from(total_len);
    sum
}

fn linux_vnet_checksum(bytes: &[u8], initial: u64) -> u16 {
    let mut sum = linux_vnet_add_words(initial, bytes);
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    sum as u16
}

fn linux_vnet_add_words(mut sum: u64, bytes: &[u8]) -> u64 {
    let mut chunks = bytes.chunks_exact(2);
    for chunk in &mut chunks {
        sum += u64::from(u16::from_be_bytes([chunk[0], chunk[1]]));
    }
    if let Some(&byte) = chunks.remainder().first() {
        sum += u64::from(byte) << 8;
    }
    sum
}

#[cfg(test)]
mod linux_vnet_tun_tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn linux_vnet_tun_env_parser_is_opt_in() {
        assert!(!linux_vnet_tun_enabled_from_env(None));
        assert!(!linux_vnet_tun_enabled_from_env(Some("")));
        assert!(!linux_vnet_tun_enabled_from_env(Some("off")));
        assert!(!linux_vnet_tun_enabled_from_env(Some("0")));
        assert!(linux_vnet_tun_enabled_from_env(Some("1")));
        assert!(linux_vnet_tun_enabled_from_env(Some("true")));
    }

    #[test]
    fn linux_vnet_plain_read_strips_virtio_header() {
        let packet = ipv4_tcp_gso_packet(16, 16, 0x10);
        let mut frame = vec![0_u8; LINUX_VIRTIO_NET_HDR_LEN + packet.len()];
        LinuxVirtioNetHdr {
            flags: 0,
            gso_type: LINUX_VIRTIO_NET_HDR_GSO_NONE,
            hdr_len: 0,
            gso_size: 0,
            csum_start: 0,
            csum_offset: 0,
        }
        .encode(&mut frame[..LINUX_VIRTIO_NET_HDR_LEN]);
        frame[LINUX_VIRTIO_NET_HDR_LEN..].copy_from_slice(&packet);

        let mut batch = Vec::new();
        let count = handle_linux_vnet_read(&mut frame, &mut batch).expect("plain vnet read");
        assert_eq!(count, 1);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].bytes.len(), packet.len());
        assert_eq!(&batch[0].bytes[..20], &packet[..20]);
    }

    #[test]
    fn linux_vnet_tcp4_gso_read_splits_into_checked_segments() {
        let packet = ipv4_tcp_gso_packet(2400, 1200, 0x18);
        let mut frame = vec![0_u8; LINUX_VIRTIO_NET_HDR_LEN + packet.len()];
        LinuxVirtioNetHdr {
            flags: LINUX_VIRTIO_NET_HDR_F_NEEDS_CSUM,
            gso_type: LINUX_VIRTIO_NET_HDR_GSO_TCPV4,
            hdr_len: 40,
            gso_size: 1200,
            csum_start: 20,
            csum_offset: 16,
        }
        .encode(&mut frame[..LINUX_VIRTIO_NET_HDR_LEN]);
        frame[LINUX_VIRTIO_NET_HDR_LEN..].copy_from_slice(&packet);

        let mut batch = Vec::new();
        let count = handle_linux_vnet_read(&mut frame, &mut batch).expect("tcp4 gso read");
        assert_eq!(count, 2);
        assert_eq!(batch.len(), 2);

        let first = &batch[0].bytes;
        let second = &batch[1].bytes;
        assert_eq!(first.len(), 1240);
        assert_eq!(second.len(), 1240);
        assert_eq!(u16::from_be_bytes([first[2], first[3]]), 1240);
        assert_eq!(u16::from_be_bytes([second[2], second[3]]), 1240);
        assert_eq!(u16::from_be_bytes([first[4], first[5]]), 0x1234);
        assert_eq!(u16::from_be_bytes([second[4], second[5]]), 0x1235);
        assert_eq!(u32::from_be_bytes([first[24], first[25], first[26], first[27]]), 1000);
        assert_eq!(
            u32::from_be_bytes([second[24], second[25], second[26], second[27]]),
            2200
        );
        assert_eq!(first[33] & LINUX_TCP_FLAG_PSH, 0);
        assert_ne!(second[33] & LINUX_TCP_FLAG_PSH, 0);
        assert_eq!(linux_vnet_checksum(&first[..20], 0), 0xffff);
        assert_eq!(linux_vnet_checksum(&second[..20], 0), 0xffff);
        assert_eq!(ipv4_transport_sum(first), 0xffff);
        assert_eq!(ipv4_transport_sum(second), 0xffff);
    }

    #[test]
    fn linux_vnet_tcp4_gro_write_coalesces_adjacent_segments() {
        let mut first = ipv4_tcp_packet(1000, 800, LINUX_TCP_FLAG_ACK);
        let mut second = ipv4_tcp_packet(1800, 600, LINUX_TCP_FLAG_ACK | LINUX_TCP_FLAG_PSH);
        nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(&mut first);
        nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(&mut second);

        let packets = vec![first, second];
        let frames = linux_vnet_prepare_write_frames(&packets);
        assert_eq!(frames.len(), 1);

        let hdr = LinuxVirtioNetHdr::decode(&frames[0]).expect("virtio header");
        assert_eq!(hdr.flags, LINUX_VIRTIO_NET_HDR_F_NEEDS_CSUM);
        assert_eq!(hdr.gso_type, LINUX_VIRTIO_NET_HDR_GSO_TCPV4);
        assert_eq!(hdr.hdr_len, 40);
        assert_eq!(hdr.gso_size, 800);
        assert_eq!(hdr.csum_start, 20);
        assert_eq!(hdr.csum_offset, 16);

        let packet = &frames[0][LINUX_VIRTIO_NET_HDR_LEN..];
        assert_eq!(packet.len(), 20 + 20 + 1400);
        assert_eq!(u16::from_be_bytes([packet[2], packet[3]]), 1440);
        assert_eq!(linux_vnet_checksum(&packet[..20], 0), 0xffff);
        assert_ne!(packet[33] & LINUX_TCP_FLAG_PSH, 0);

        let pseudo = linux_vnet_pseudo_header_sum(
            LINUX_IPPROTO_TCP,
            &packet[12..16],
            &packet[16..20],
            (packet.len() - 20) as u16,
        );
        let expected_partial = !linux_vnet_checksum(&[], pseudo);
        assert_eq!(
            u16::from_be_bytes([packet[36], packet[37]]),
            expected_partial
        );
    }

    #[test]
    fn linux_vnet_tcp4_gro_write_keeps_sequence_gap_separate() {
        let mut first = ipv4_tcp_packet(1000, 800, LINUX_TCP_FLAG_ACK);
        let mut second = ipv4_tcp_packet(2000, 600, LINUX_TCP_FLAG_ACK);
        nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(&mut first);
        nostr_vpn_core::packet_checksums::finalize_ipv4_transport_checksum(&mut second);

        let packets = vec![first, second];
        let frames = linux_vnet_prepare_write_frames(&packets);
        assert_eq!(frames.len(), 2);
        for frame in frames {
            let hdr = LinuxVirtioNetHdr::decode(&frame).expect("virtio header");
            assert_eq!(hdr.gso_type, LINUX_VIRTIO_NET_HDR_GSO_NONE);
            assert_eq!(hdr.gso_size, 0);
        }
    }

    fn ipv4_tcp_gso_packet(payload_len: usize, gso_size: usize, flags: u8) -> Vec<u8> {
        let packet = ipv4_tcp_packet(1000, payload_len, flags);
        assert_eq!(payload_len % gso_size, 0);
        packet
    }

    fn ipv4_tcp_packet(seq: u32, payload_len: usize, flags: u8) -> Vec<u8> {
        let total_len = 20 + 20 + payload_len;
        let mut packet = vec![0_u8; total_len];
        packet[0] = 0x45;
        packet[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
        packet[4..6].copy_from_slice(&0x1234_u16.to_be_bytes());
        packet[8] = 64;
        packet[9] = LINUX_IPPROTO_TCP;
        packet[12..16].copy_from_slice(&Ipv4Addr::new(10, 44, 0, 1).octets());
        packet[16..20].copy_from_slice(&Ipv4Addr::new(10, 44, 0, 2).octets());
        packet[20..22].copy_from_slice(&443_u16.to_be_bytes());
        packet[22..24].copy_from_slice(&45172_u16.to_be_bytes());
        packet[24..28].copy_from_slice(&seq.to_be_bytes());
        packet[28..32].copy_from_slice(&777_u32.to_be_bytes());
        packet[32] = 5 << 4;
        packet[33] = flags;
        packet[34..36].copy_from_slice(&65535_u16.to_be_bytes());
        for i in 0..payload_len {
            packet[40 + i] = (i % 251) as u8;
        }
        packet
    }

    fn ipv4_transport_sum(packet: &[u8]) -> u16 {
        let transport_len = packet.len() - 20;
        let pseudo = linux_vnet_pseudo_header_sum(
            LINUX_IPPROTO_TCP,
            &packet[12..16],
            &packet[16..20],
            transport_len as u16,
        );
        linux_vnet_checksum(&packet[20..], pseudo)
    }
}
