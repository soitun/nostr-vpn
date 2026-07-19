use std::{collections::HashMap, io, mem, thread};

use tokio::sync::mpsc;

const NETLINK_HEADER_LEN: usize = 16;
const IFADDR_MESSAGE_LEN: usize = 8;
const ROUTE_ATTRIBUTE_HEADER_LEN: usize = 4;
const NETLINK_ALIGNMENT: usize = 4;
const NETLINK_ATTRIBUTE_TYPE_MASK: u16 = 0x3fff;
const NETLINK_MESSAGE_NOOP: u16 = 1;
const NETLINK_MESSAGE_DONE: u16 = 3;

#[derive(Debug, Eq, Hash, PartialEq)]
struct LinuxInterfaceAddressKey {
    family: u8,
    prefix_len: u8,
    interface_index: u32,
    address: Vec<u8>,
    local: Vec<u8>,
}

#[derive(Default)]
struct LinuxRouteEventDeduper {
    interface_addresses: HashMap<LinuxInterfaceAddressKey, (u8, u32)>,
}

impl LinuxRouteEventDeduper {
    // Network managers may renew an unchanged address by sending RTM_NEWADDR
    // with only IFA_CACHEINFO timestamps changed. Those lifetime refreshes do
    // not affect FIPS paths and must not trigger a full route reconciliation.
    fn has_meaningful_change(&mut self, packet: &[u8]) -> bool {
        let mut offset = 0;
        let mut changed = false;
        while offset < packet.len() {
            let Some(header) = packet.get(offset..offset + NETLINK_HEADER_LEN) else {
                return true;
            };
            let Some(message_len) = read_ne_u32(header).map(|len| len as usize) else {
                return true;
            };
            let Some(message_end) = offset.checked_add(message_len) else {
                return true;
            };
            if message_len < NETLINK_HEADER_LEN || message_end > packet.len() {
                return true;
            }
            let message_type = read_ne_u16(&header[4..]).unwrap_or_default();
            let payload = &packet[offset + NETLINK_HEADER_LEN..message_end];
            match message_type {
                libc::RTM_NEWADDR => {
                    let Some((key, state)) = parse_interface_address(payload) else {
                        return true;
                    };
                    if self.interface_addresses.insert(key, state) != Some(state) {
                        changed = true;
                    }
                }
                libc::RTM_DELADDR => {
                    let Some((key, _)) = parse_interface_address(payload) else {
                        return true;
                    };
                    self.interface_addresses.remove(&key);
                    changed = true;
                }
                NETLINK_MESSAGE_NOOP | NETLINK_MESSAGE_DONE => {}
                _ => changed = true,
            }
            offset += align_netlink(message_len);
        }
        changed
    }
}

fn parse_interface_address(payload: &[u8]) -> Option<(LinuxInterfaceAddressKey, (u8, u32))> {
    let header = payload.get(..IFADDR_MESSAGE_LEN)?;
    let family = header[0];
    let prefix_len = header[1];
    let mut flags = u32::from(header[2]);
    let scope = header[3];
    let interface_index = read_ne_u32(&header[4..])?;
    let mut address = Vec::new();
    let mut local = Vec::new();
    let mut offset = IFADDR_MESSAGE_LEN;

    while offset < payload.len() {
        let attribute_header = payload.get(offset..offset + ROUTE_ATTRIBUTE_HEADER_LEN)?;
        let attribute_len = usize::from(read_ne_u16(attribute_header)?);
        let attribute_end = offset.checked_add(attribute_len)?;
        if attribute_len < ROUTE_ATTRIBUTE_HEADER_LEN || attribute_end > payload.len() {
            return None;
        }
        let attribute_type = read_ne_u16(&attribute_header[2..])? & NETLINK_ATTRIBUTE_TYPE_MASK;
        let value = &payload[offset + ROUTE_ATTRIBUTE_HEADER_LEN..attribute_end];
        match attribute_type {
            libc::IFA_ADDRESS => address.extend_from_slice(value),
            libc::IFA_LOCAL => local.extend_from_slice(value),
            libc::IFA_FLAGS => flags = read_ne_u32(value)?,
            _ => {}
        }
        offset += align_netlink(attribute_len);
    }

    if address.is_empty() && local.is_empty() {
        return None;
    }
    Some((
        LinuxInterfaceAddressKey {
            family,
            prefix_len,
            interface_index,
            address,
            local,
        },
        (scope, flags),
    ))
}

fn read_ne_u16(bytes: &[u8]) -> Option<u16> {
    Some(u16::from_ne_bytes(bytes.get(..2)?.try_into().ok()?))
}

fn read_ne_u32(bytes: &[u8]) -> Option<u32> {
    Some(u32::from_ne_bytes(bytes.get(..4)?.try_into().ok()?))
}

const fn align_netlink(len: usize) -> usize {
    (len + NETLINK_ALIGNMENT - 1) & !(NETLINK_ALIGNMENT - 1)
}

pub(crate) fn spawn_linux_route_change_monitor() -> Option<mpsc::Receiver<()>> {
    let fd = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_RAW, libc::NETLINK_ROUTE) };
    if fd < 0 {
        eprintln!(
            "daemon: failed to open Linux netlink route monitor socket: {}",
            io::Error::last_os_error()
        );
        return None;
    }

    let groups = (libc::RTMGRP_LINK
        | libc::RTMGRP_IPV4_IFADDR
        | libc::RTMGRP_IPV6_IFADDR
        | libc::RTMGRP_IPV4_ROUTE
        | libc::RTMGRP_IPV6_ROUTE) as u32;
    let mut addr = unsafe { mem::zeroed::<libc::sockaddr_nl>() };
    addr.nl_family = libc::AF_NETLINK as libc::sa_family_t;
    addr.nl_pid = 0;
    addr.nl_groups = groups;
    let bind_result = unsafe {
        libc::bind(
            fd,
            (&addr as *const libc::sockaddr_nl).cast::<libc::sockaddr>(),
            mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
        )
    };
    if bind_result < 0 {
        let error = io::Error::last_os_error();
        unsafe {
            libc::close(fd);
        }
        eprintln!("daemon: failed to bind Linux netlink route monitor: {error}");
        return None;
    }

    let (tx, rx) = mpsc::channel(1);
    let spawn_result = thread::Builder::new()
        .name("nvpn-linux-route-monitor".to_string())
        .spawn(move || {
            let _fd = LinuxRouteMonitorFd(fd);
            let mut buf = [0_u8; 8192];
            let mut deduper = LinuxRouteEventDeduper::default();
            loop {
                let read = unsafe {
                    libc::recv(fd, buf.as_mut_ptr().cast::<libc::c_void>(), buf.len(), 0)
                };
                if read < 0 {
                    eprintln!(
                        "daemon: Linux netlink route monitor read failed: {}",
                        io::Error::last_os_error()
                    );
                    break;
                }
                if read == 0 {
                    continue;
                }
                if !deduper.has_meaningful_change(&buf[..read.unsigned_abs()]) {
                    continue;
                }
                match tx.try_send(()) {
                    Ok(()) | Err(mpsc::error::TrySendError::Full(())) => {}
                    Err(mpsc::error::TrySendError::Closed(())) => break,
                }
            }
        });

    match spawn_result {
        Ok(_) => Some(rx),
        Err(error) => {
            unsafe {
                libc::close(fd);
            }
            eprintln!("daemon: failed to spawn Linux netlink route monitor: {error}");
            None
        }
    }
}

struct LinuxRouteMonitorFd(libc::c_int);

impl Drop for LinuxRouteMonitorFd {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_identical_address_notifications_are_suppressed() {
        let mut deduper = LinuxRouteEventDeduper::default();
        let first = address_message(libc::RTM_NEWADDR, [10, 44, 155, 172], 10);
        let refreshed_lifetime = address_message(libc::RTM_NEWADDR, [10, 44, 155, 172], 20);

        assert!(deduper.has_meaningful_change(&first));
        assert!(!deduper.has_meaningful_change(&refreshed_lifetime));
    }

    #[test]
    fn removed_or_changed_addresses_remain_meaningful() {
        let mut deduper = LinuxRouteEventDeduper::default();
        let first = address_message(libc::RTM_NEWADDR, [10, 44, 155, 172], 10);
        let removed = address_message(libc::RTM_DELADDR, [10, 44, 155, 172], 11);
        let readded = address_message(libc::RTM_NEWADDR, [10, 44, 155, 172], 12);

        assert!(deduper.has_meaningful_change(&first));
        assert!(deduper.has_meaningful_change(&removed));
        assert!(deduper.has_meaningful_change(&readded));
    }

    #[test]
    fn route_notifications_remain_meaningful() {
        let mut packet = vec![0_u8; NETLINK_HEADER_LEN];
        packet[..4].copy_from_slice(&(NETLINK_HEADER_LEN as u32).to_ne_bytes());
        packet[4..6].copy_from_slice(&libc::RTM_NEWROUTE.to_ne_bytes());

        assert!(LinuxRouteEventDeduper::default().has_meaningful_change(&packet));
    }

    fn address_message(message_type: u16, address: [u8; 4], cache_stamp: u32) -> Vec<u8> {
        let mut payload = vec![libc::AF_INET as u8, 32, 0, 0];
        payload.extend_from_slice(&1040_u32.to_ne_bytes());
        append_attribute(&mut payload, libc::IFA_LOCAL, &address);
        let mut cache_info = [0xff; 16];
        cache_info[8..12].copy_from_slice(&cache_stamp.to_ne_bytes());
        append_attribute(&mut payload, libc::IFA_CACHEINFO, &cache_info);

        let message_len = NETLINK_HEADER_LEN + payload.len();
        let mut message = vec![0_u8; NETLINK_HEADER_LEN];
        message[..4].copy_from_slice(&(message_len as u32).to_ne_bytes());
        message[4..6].copy_from_slice(&message_type.to_ne_bytes());
        message.extend_from_slice(&payload);
        message
    }

    fn append_attribute(message: &mut Vec<u8>, attribute_type: u16, value: &[u8]) {
        let attribute_len = ROUTE_ATTRIBUTE_HEADER_LEN + value.len();
        message.extend_from_slice(&(attribute_len as u16).to_ne_bytes());
        message.extend_from_slice(&attribute_type.to_ne_bytes());
        message.extend_from_slice(value);
        message.resize(
            message.len() + align_netlink(attribute_len) - attribute_len,
            0,
        );
    }
}
