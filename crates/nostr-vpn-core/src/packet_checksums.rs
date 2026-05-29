//! Helpers for packets that have crossed a userspace tunnel boundary.
//!
//! Linux can hand TUN readers packets whose TCP/UDP checksum still relies on
//! kernel checksum metadata. That metadata is lost when we carry the raw IP
//! bytes over FIPS, so receivers may drop otherwise valid TCP packets. Finalize
//! checksums before serializing packets onto the overlay or writing them back to
//! an OS-owned TUN.

#![allow(clippy::cast_possible_truncation)]

/// Recompute IPv4 TCP/UDP checksums in-place.
///
/// This is intentionally conservative: it only touches complete, unfragmented
/// IPv4 TCP/UDP packets. IPv4 UDP packets with checksum 0 keep checksum 0,
/// which is a valid "no checksum" marker.
pub fn finalize_ipv4_transport_checksum(packet: &mut [u8]) {
    if packet.len() < 20 || packet[0] >> 4 != 4 {
        return;
    }

    let ihl = usize::from(packet[0] & 0x0f) * 4;
    if ihl < 20 || packet.len() < ihl {
        return;
    }

    let total_len = usize::from(u16::from_be_bytes([packet[2], packet[3]]));
    if total_len < ihl || total_len > packet.len() {
        return;
    }

    let fragment = u16::from_be_bytes([packet[6], packet[7]]);
    let has_more_fragments = fragment & 0x2000 != 0;
    let fragment_offset = fragment & 0x1fff;
    if has_more_fragments || fragment_offset != 0 {
        return;
    }

    let protocol = packet[9];
    let transport_len = total_len - ihl;
    match protocol {
        6 if transport_len >= 20 => {
            packet[ihl + 16] = 0;
            packet[ihl + 17] = 0;
            let checksum = ipv4_transport_checksum(packet, ihl, transport_len, protocol);
            packet[ihl + 16..ihl + 18].copy_from_slice(&checksum.to_be_bytes());
        }
        17 if transport_len >= 8 => {
            let old = u16::from_be_bytes([packet[ihl + 6], packet[ihl + 7]]);
            if old == 0 {
                return;
            }
            packet[ihl + 6] = 0;
            packet[ihl + 7] = 0;
            let checksum = ipv4_transport_checksum(packet, ihl, transport_len, protocol);
            let checksum = if checksum == 0 { 0xffff } else { checksum };
            packet[ihl + 6..ihl + 8].copy_from_slice(&checksum.to_be_bytes());
        }
        _ => {}
    }
}

fn ipv4_transport_checksum(packet: &[u8], ihl: usize, transport_len: usize, protocol: u8) -> u16 {
    let mut sum = 0_u32;
    sum = add_words(sum, &packet[12..16]);
    sum = add_words(sum, &packet[16..20]);
    sum += u32::from(protocol);
    sum += transport_len as u32;
    sum = add_words(sum, &packet[ihl..ihl + transport_len]);
    finalize_sum(sum)
}

fn add_words(mut sum: u32, bytes: &[u8]) -> u32 {
    let mut chunks = bytes.chunks_exact(2);
    for chunk in &mut chunks {
        sum += u32::from(u16::from_be_bytes([chunk[0], chunk[1]]));
    }
    if let Some(&byte) = chunks.remainder().first() {
        sum += u32::from(byte) << 8;
    }
    sum
}

fn finalize_sum(mut sum: u32) -> u16 {
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn tcp_packet(src: Ipv4Addr, dst: Ipv4Addr) -> Vec<u8> {
        let total_len = 20 + 20;
        let mut packet = vec![0_u8; total_len];
        packet[0] = 0x45;
        packet[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
        packet[8] = 64;
        packet[9] = 6;
        packet[12..16].copy_from_slice(&src.octets());
        packet[16..20].copy_from_slice(&dst.octets());
        packet[20..22].copy_from_slice(&443_u16.to_be_bytes());
        packet[22..24].copy_from_slice(&45172_u16.to_be_bytes());
        packet[32] = 0x50;
        packet[33] = 0x12;
        packet[34..36].copy_from_slice(&65535_u16.to_be_bytes());
        packet[36..38].copy_from_slice(&0xa77d_u16.to_be_bytes());
        packet
    }

    fn udp_packet(src: Ipv4Addr, dst: Ipv4Addr) -> Vec<u8> {
        let payload = b"hello";
        let total_len = 20 + 8 + payload.len();
        let mut packet = vec![0_u8; total_len];
        packet[0] = 0x45;
        packet[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
        packet[8] = 64;
        packet[9] = 17;
        packet[12..16].copy_from_slice(&src.octets());
        packet[16..20].copy_from_slice(&dst.octets());
        packet[20..22].copy_from_slice(&5353_u16.to_be_bytes());
        packet[22..24].copy_from_slice(&5353_u16.to_be_bytes());
        packet[24..26].copy_from_slice(&(8_u16 + payload.len() as u16).to_be_bytes());
        packet[26..28].copy_from_slice(&0xbeef_u16.to_be_bytes());
        packet[28..].copy_from_slice(payload);
        packet
    }

    #[test]
    fn finalizes_ipv4_tcp_checksum() {
        let mut packet = tcp_packet(
            Ipv4Addr::new(172, 66, 147, 243),
            Ipv4Addr::new(10, 44, 80, 2),
        );
        finalize_ipv4_transport_checksum(&mut packet);
        let checksum = u16::from_be_bytes([packet[36], packet[37]]);

        assert_ne!(checksum, 0xa77d);
        assert_eq!(ipv4_transport_checksum(&packet, 20, 20, 6), 0);
    }

    #[test]
    fn finalizes_ipv4_udp_checksum_when_present() {
        let mut packet = udp_packet(
            Ipv4Addr::new(10, 44, 80, 2),
            Ipv4Addr::new(10, 44, 204, 215),
        );
        finalize_ipv4_transport_checksum(&mut packet);

        assert_eq!(
            ipv4_transport_checksum(&packet, 20, packet.len() - 20, 17),
            0
        );
    }

    #[test]
    fn preserves_ipv4_udp_zero_checksum() {
        let mut packet = udp_packet(
            Ipv4Addr::new(10, 44, 80, 2),
            Ipv4Addr::new(10, 44, 204, 215),
        );
        packet[26] = 0;
        packet[27] = 0;
        finalize_ipv4_transport_checksum(&mut packet);

        assert_eq!(&packet[26..28], &[0, 0]);
    }
}
