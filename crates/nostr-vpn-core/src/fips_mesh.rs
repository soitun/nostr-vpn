use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use anyhow::{Context, Result};
use nostr_sdk::prelude::{PublicKey, ToBech32};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::normalize_nostr_pubkey;
use crate::data_plane::{MeshPeerStatus, PrivatePacket};
#[cfg(feature = "paid-exit")]
use crate::paid_route_store::PaidRouteSellerAdmission;
use crate::paid_routes::PaidRouteAccessState;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FipsMeshPeerConfig {
    pub participant_pubkey: String,
    pub endpoint_npub: String,
    pub allowed_ips: Vec<String>,
}

impl FipsMeshPeerConfig {
    pub fn from_participant_pubkey(
        participant_pubkey: impl AsRef<str>,
        allowed_ips: Vec<String>,
    ) -> Result<Self> {
        let participant_pubkey = normalize_nostr_pubkey(participant_pubkey.as_ref())?;
        let endpoint_npub = npub_for_pubkey_hex(&participant_pubkey)?;

        Ok(Self {
            participant_pubkey,
            endpoint_npub,
            allowed_ips,
        })
    }

    pub fn advertises_default_route(&self) -> bool {
        self.allowed_ips
            .iter()
            .any(|route| matches!(route.trim(), "0.0.0.0/0" | "::/0"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingFipsPacket {
    pub participant_pubkey: String,
    pub endpoint_npub: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FipsPaidRouteAdmission {
    pub participant_pubkey: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_ips: Vec<String>,
    pub allow_routing: bool,
    pub state: PaidRouteAccessState,
    pub amount_due_msat: u64,
    pub paid_msat: u64,
    pub unpaid_msat: u64,
    pub expires_at_unix: u64,
    pub updated_at_unix: u64,
}

#[cfg(feature = "paid-exit")]
impl From<PaidRouteSellerAdmission> for FipsPaidRouteAdmission {
    fn from(value: PaidRouteSellerAdmission) -> Self {
        Self {
            participant_pubkey: value.buyer_pubkey,
            session_id: value.session_id,
            allowed_ips: Vec::new(),
            allow_routing: value.allow_routing,
            state: value.state,
            amount_due_msat: value.amount_due_msat,
            paid_msat: value.paid_msat,
            unpaid_msat: value.unpaid_msat,
            expires_at_unix: value.expires_at_unix,
            updated_at_unix: value.updated_at_unix,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct RoutedFipsPacket<'a> {
    pub participant_pubkey: &'a str,
    pub participant_pubkey_bytes: Option<&'a [u8; 32]>,
    pub endpoint_pubkey: &'a [u8; 32],
    pub endpoint_node_addr: &'a [u8; 16],
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoutedFipsPeer<'a> {
    pub participant_pubkey: &'a str,
    pub participant_pubkey_bytes: Option<&'a [u8; 32]>,
    pub endpoint_pubkey: &'a [u8; 32],
    pub endpoint_node_addr: &'a [u8; 16],
}

#[derive(Debug, PartialEq, Eq)]
pub struct AcceptedFipsPacket<'a> {
    pub source_pubkey: &'a str,
    pub source_pubkey_bytes: Option<&'a [u8; 32]>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct FipsMeshRuntime {
    peers: Vec<FipsMeshPeerRuntime>,
    local_routes: Vec<IpRoute>,
    paid_route_admissions: HashMap<[u8; 32], FipsPaidRouteAdmission>,
    paid_route_peers: Vec<FipsMeshPeerRuntime>,
    participant_peer_index: HashMap<[u8; 32], usize>,
    endpoint_pubkey_peer_index: HashMap<[u8; 32], usize>,
    endpoint_node_addr_peer_index: HashMap<[u8; 16], usize>,
    exact_route_peer_index: HashMap<IpAddr, ExactRouteMatch>,
    prefix_v4_route_peer_index: Vec<IndexedIpRoute>,
    prefix_v6_route_peer_index: Vec<IndexedIpRoute>,
}

#[derive(Debug, Clone)]
struct FipsMeshPeerRuntime {
    participant_pubkey: Option<[u8; 32]>,
    participant_pubkey_hex: String,
    endpoint_npub: Option<String>,
    endpoint_pubkey: Option<[u8; 32]>,
    endpoint_node_addr: Option<[u8; 16]>,
    routes: Vec<IpRoute>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IpRoute {
    network: IpAddr,
    prefix_len: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExactRouteMatch {
    Peer(usize),
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IndexedIpRoute {
    peer_index: usize,
    route: IpRoute,
}

impl FipsMeshRuntime {
    pub fn new(peers: Vec<FipsMeshPeerConfig>) -> Self {
        Self::with_local_routes(peers, Vec::new())
    }

    pub fn with_local_routes(
        peers: Vec<FipsMeshPeerConfig>,
        local_allowed_ips: Vec<String>,
    ) -> Self {
        Self::with_local_routes_internal(peers, local_allowed_ips, Vec::new())
    }

    pub fn with_local_routes_and_paid_route_admissions(
        peers: Vec<FipsMeshPeerConfig>,
        local_allowed_ips: Vec<String>,
        paid_route_admissions: Vec<FipsPaidRouteAdmission>,
    ) -> Self {
        Self::with_local_routes_internal(peers, local_allowed_ips, paid_route_admissions)
    }

    fn with_local_routes_internal(
        peers: Vec<FipsMeshPeerConfig>,
        local_allowed_ips: Vec<String>,
        paid_route_admissions: Vec<FipsPaidRouteAdmission>,
    ) -> Self {
        let peers = peers
            .into_iter()
            .map(|peer| {
                let routes = peer
                    .allowed_ips
                    .iter()
                    .filter_map(|route| IpRoute::parse(route))
                    .collect();

                let endpoint_pubkey = parse_nostr_pubkey_bytes(&peer.endpoint_npub);
                let endpoint_node_addr = endpoint_pubkey.map(endpoint_node_addr_from_pubkey_bytes);
                let endpoint_npub =
                    endpoint_pubkey.and_then(|pubkey| npub_for_pubkey_bytes(&pubkey).ok());
                let (participant_pubkey, participant_pubkey_hex) =
                    runtime_participant_pubkey(&peer.participant_pubkey);

                FipsMeshPeerRuntime {
                    participant_pubkey,
                    participant_pubkey_hex,
                    endpoint_npub,
                    endpoint_pubkey,
                    endpoint_node_addr,
                    routes,
                }
            })
            .collect::<Vec<_>>();
        let participant_peer_index = participant_peer_index(&peers);
        let (endpoint_pubkey_peer_index, endpoint_node_addr_peer_index) =
            endpoint_peer_indexes(&peers);
        let exact_route_peer_index = exact_route_peer_index(&peers);
        let (prefix_v4_route_peer_index, prefix_v6_route_peer_index) =
            prefix_route_peer_indexes(&peers);
        let local_routes = local_allowed_ips
            .iter()
            .filter_map(|route| IpRoute::parse(route))
            .collect();

        let paid_route_admissions = normalize_paid_route_admissions(paid_route_admissions);
        let paid_route_peers = paid_route_peers_from_admissions(&paid_route_admissions);

        Self {
            peers,
            local_routes,
            paid_route_admissions,
            paid_route_peers,
            participant_peer_index,
            endpoint_pubkey_peer_index,
            endpoint_node_addr_peer_index,
            exact_route_peer_index,
            prefix_v4_route_peer_index,
            prefix_v6_route_peer_index,
        }
    }

    pub fn replace_paid_route_admissions(
        &mut self,
        paid_route_admissions: Vec<FipsPaidRouteAdmission>,
    ) {
        self.paid_route_admissions = normalize_paid_route_admissions(paid_route_admissions);
        self.paid_route_peers = paid_route_peers_from_admissions(&self.paid_route_admissions);
    }

    pub fn route_outbound_packet(&self, packet: &[u8]) -> Option<OutgoingFipsPacket> {
        let peer = self.route_outbound_peer(packet)?;
        let endpoint_npub = peer.endpoint_npub.clone()?;

        Some(OutgoingFipsPacket {
            participant_pubkey: peer.participant_pubkey_hex.clone(),
            endpoint_npub,
            bytes: packet.to_vec(),
        })
    }

    pub fn route_outbound_packet_owned(&self, packet: Vec<u8>) -> Option<OutgoingFipsPacket> {
        let peer = self.route_outbound_peer(&packet)?;
        let endpoint_npub = peer.endpoint_npub.clone()?;

        Some(OutgoingFipsPacket {
            participant_pubkey: peer.participant_pubkey_hex.clone(),
            endpoint_npub,
            bytes: packet,
        })
    }

    pub fn route_outbound_packet_peer<'a>(&'a self, packet: &[u8]) -> Option<RoutedFipsPeer<'a>> {
        let peer = self.route_outbound_peer(packet)?;

        Some(RoutedFipsPeer {
            participant_pubkey: &peer.participant_pubkey_hex,
            participant_pubkey_bytes: peer.participant_pubkey.as_ref(),
            endpoint_pubkey: peer.endpoint_pubkey.as_ref()?,
            endpoint_node_addr: peer.endpoint_node_addr.as_ref()?,
        })
    }

    pub fn route_outbound_packet_with_peer<'a>(
        &'a self,
        packet: &[u8],
    ) -> Option<RoutedFipsPacket<'a>> {
        let peer = self.route_outbound_peer(packet)?;

        Some(RoutedFipsPacket {
            participant_pubkey: &peer.participant_pubkey_hex,
            participant_pubkey_bytes: peer.participant_pubkey.as_ref(),
            endpoint_pubkey: peer.endpoint_pubkey.as_ref()?,
            endpoint_node_addr: peer.endpoint_node_addr.as_ref()?,
            bytes: packet.to_vec(),
        })
    }

    pub fn route_outbound_packet_owned_with_peer<'a>(
        &'a self,
        packet: Vec<u8>,
    ) -> Option<RoutedFipsPacket<'a>> {
        let destination = packet_destination(&packet)?;
        self.route_outbound_packet_owned_with_peer_to_destination(packet, destination)
    }

    pub fn route_outbound_packet_owned_with_peer_to_destination<'a>(
        &'a self,
        packet: Vec<u8>,
        destination: IpAddr,
    ) -> Option<RoutedFipsPacket<'a>> {
        let peer = self.select_peer_for_ip(destination)?;

        Some(RoutedFipsPacket {
            participant_pubkey: &peer.participant_pubkey_hex,
            participant_pubkey_bytes: peer.participant_pubkey.as_ref(),
            endpoint_pubkey: peer.endpoint_pubkey.as_ref()?,
            endpoint_node_addr: peer.endpoint_node_addr.as_ref()?,
            bytes: packet,
        })
    }

    fn route_outbound_peer(&self, packet: &[u8]) -> Option<&FipsMeshPeerRuntime> {
        let destination = packet_destination(packet)?;
        self.select_peer_for_ip(destination)
    }

    pub fn receive_endpoint_data(
        &self,
        source_npub: Option<&str>,
        data: &[u8],
    ) -> Option<PrivatePacket> {
        let peer = self.admit_endpoint_data(source_npub, data)?;

        Some(PrivatePacket {
            source_pubkey: peer.participant_pubkey_hex.clone(),
            bytes: data.to_vec(),
        })
    }

    pub fn receive_endpoint_data_owned(
        &self,
        source_npub: Option<&str>,
        data: Vec<u8>,
    ) -> Option<PrivatePacket> {
        let peer = self.admit_endpoint_data(source_npub, &data)?;

        Some(PrivatePacket {
            source_pubkey: peer.participant_pubkey_hex.clone(),
            bytes: data,
        })
    }

    pub fn receive_endpoint_data_owned_with_source<'a>(
        &'a self,
        source_npub: Option<&str>,
        data: Vec<u8>,
    ) -> Option<AcceptedFipsPacket<'a>> {
        let peer = self.admit_endpoint_data(source_npub, &data)?;

        Some(AcceptedFipsPacket {
            source_pubkey: &peer.participant_pubkey_hex,
            source_pubkey_bytes: peer.participant_pubkey.as_ref(),
            bytes: data,
        })
    }

    pub fn receive_endpoint_data_from_node_addr(
        &self,
        source_node_addr: &[u8; 16],
        data: &[u8],
    ) -> Option<PrivatePacket> {
        let peer = self.admit_endpoint_data_from_node_addr(source_node_addr, data)?;

        Some(PrivatePacket {
            source_pubkey: peer.participant_pubkey_hex.clone(),
            bytes: data.to_vec(),
        })
    }

    pub fn receive_endpoint_data_owned_from_node_addr(
        &self,
        source_node_addr: &[u8; 16],
        data: Vec<u8>,
    ) -> Option<PrivatePacket> {
        let peer = self.admit_endpoint_data_from_node_addr(source_node_addr, &data)?;

        Some(PrivatePacket {
            source_pubkey: peer.participant_pubkey_hex.clone(),
            bytes: data,
        })
    }

    pub fn receive_endpoint_data_owned_with_source_node_addr<'a>(
        &'a self,
        source_node_addr: &[u8; 16],
        data: Vec<u8>,
    ) -> Option<AcceptedFipsPacket<'a>> {
        let peer = self.admit_endpoint_data_from_node_addr(source_node_addr, &data)?;

        Some(AcceptedFipsPacket {
            source_pubkey: &peer.participant_pubkey_hex,
            source_pubkey_bytes: peer.participant_pubkey.as_ref(),
            bytes: data,
        })
    }

    fn admit_endpoint_data(
        &self,
        source_npub: Option<&str>,
        data: &[u8],
    ) -> Option<&FipsMeshPeerRuntime> {
        let source_pubkey = parse_nostr_pubkey_bytes(source_npub?)?;
        let packet_source = packet_source(data)?;
        let peer = self.select_peer_for_ip(packet_source)?;
        if peer.endpoint_pubkey.as_ref()? != &source_pubkey {
            return None;
        }
        let packet_destination = packet_destination(data)?;
        if !self.peer_allows_inbound_destination(peer, packet_destination) {
            return None;
        }
        Some(peer)
    }

    fn admit_endpoint_data_from_node_addr(
        &self,
        source_node_addr: &[u8; 16],
        data: &[u8],
    ) -> Option<&FipsMeshPeerRuntime> {
        let packet_source = packet_source(data)?;
        let peer = self.select_peer_for_ip(packet_source)?;
        if peer.endpoint_node_addr.as_ref()? != source_node_addr {
            return None;
        }
        if !self.local_routes.is_empty() {
            let packet_destination = packet_destination(data)?;
            if !self
                .local_routes
                .iter()
                .any(|route| route.matches(packet_destination))
            {
                return None;
            }
        }
        Some(peer)
    }

    pub fn peer_statuses(&self) -> Vec<MeshPeerStatus> {
        self.peers
            .iter()
            .map(|peer| MeshPeerStatus {
                pubkey: peer.participant_pubkey_hex.clone(),
                connected: false,
                endpoint_npub: peer.endpoint_npub.clone().unwrap_or_default(),
                transport_addr: None,
                transport_type: None,
                srtt_ms: None,
                srtt_age_ms: None,
                link_packets_sent: 0,
                link_packets_recv: 0,
                link_bytes_sent: 0,
                link_bytes_recv: 0,
                rekey_in_progress: false,
                rekey_draining: false,
                current_k_bit: None,
                last_outbound_route: None,
                direct_probe_pending: false,
                direct_probe_after_ms: None,
                direct_probe_retry_count: 0,
                direct_probe_auto_reconnect: false,
                direct_probe_expires_at_ms: None,
                nostr_traversal_consecutive_failures: 0,
                nostr_traversal_in_cooldown: false,
                nostr_traversal_cooldown_until_ms: None,
                nostr_traversal_last_observed_skew_ms: None,
                last_seen_at: None,
                last_control_seen_at: None,
                last_data_seen_at: None,
                tx_bytes: 0,
                rx_bytes: 0,
                error: Some("fips link pending".to_string()),
            })
            .collect()
    }

    pub fn participant_for_endpoint_npub(&self, endpoint_npub: &str) -> Option<String> {
        let source_pubkey = parse_nostr_pubkey_bytes(endpoint_npub)?;
        let peer_index = *self.endpoint_pubkey_peer_index.get(&source_pubkey)?;
        self.peers
            .get(peer_index)
            .map(|peer| peer.participant_pubkey_hex.clone())
    }

    pub fn participant_for_endpoint_node_addr(
        &self,
        endpoint_node_addr: &[u8; 16],
    ) -> Option<String> {
        let peer_index = *self.endpoint_node_addr_peer_index.get(endpoint_node_addr)?;
        self.peers
            .get(peer_index)
            .map(|peer| peer.participant_pubkey_hex.clone())
    }

    pub fn participant_pubkey_bytes_for_endpoint_node_addr(
        &self,
        endpoint_node_addr: &[u8; 16],
    ) -> Option<[u8; 32]> {
        let peer_index = *self.endpoint_node_addr_peer_index.get(endpoint_node_addr)?;
        self.peers
            .get(peer_index)
            .and_then(|peer| peer.participant_pubkey)
    }

    pub fn peer_endpoint_npub(&self, participant_pubkey: &str) -> Option<String> {
        let participant_pubkey = parse_nostr_pubkey_bytes(participant_pubkey)?;
        let peer_index = *self.participant_peer_index.get(&participant_pubkey)?;
        self.peers
            .get(peer_index)
            .and_then(|peer| peer.endpoint_npub.clone())
    }

    pub fn peer_endpoint_node_addr(&self, participant_pubkey: &str) -> Option<[u8; 16]> {
        let participant_pubkey = parse_nostr_pubkey_bytes(participant_pubkey)?;
        self.peer_endpoint_node_addr_for_participant_pubkey_bytes(&participant_pubkey)
    }

    pub fn peer_endpoint_node_addr_for_participant_pubkey_bytes(
        &self,
        participant_pubkey: &[u8; 32],
    ) -> Option<[u8; 16]> {
        self.peers
            .get(*self.participant_peer_index.get(participant_pubkey)?)
            .and_then(|peer| peer.endpoint_node_addr)
    }

    pub fn peer_pubkeys(&self) -> Vec<String> {
        self.peers
            .iter()
            .map(|peer| peer.participant_pubkey_hex.clone())
            .collect()
    }

    fn select_peer_for_ip(&self, destination: IpAddr) -> Option<&FipsMeshPeerRuntime> {
        if let Some(route_match) = self.exact_route_peer_index.get(&destination) {
            return match *route_match {
                ExactRouteMatch::Peer(peer_index) => self.peers.get(peer_index),
                ExactRouteMatch::Ambiguous => None,
            };
        }

        let prefix_routes = match destination {
            IpAddr::V4(_) => &self.prefix_v4_route_peer_index,
            IpAddr::V6(_) => &self.prefix_v6_route_peer_index,
        };
        let mut best_peer_index = None;
        let mut best_prefix = None;
        let mut ambiguous = false;

        for candidate in prefix_routes {
            if best_prefix.is_some_and(|prefix| candidate.route.prefix_len < prefix) {
                break;
            }
            if !candidate.route.matches(destination) {
                continue;
            }

            let Some(peer) = self.peers.get(candidate.peer_index) else {
                continue;
            };
            match best_peer_index {
                None => {
                    best_peer_index = Some(candidate.peer_index);
                    best_prefix = Some(candidate.route.prefix_len);
                    ambiguous = false;
                }
                Some(best_index)
                    if best_prefix == Some(candidate.route.prefix_len)
                        && self
                            .peers
                            .get(best_index)
                            .is_some_and(|best_peer| !same_participant(best_peer, peer)) =>
                {
                    ambiguous = true;
                }
                Some(_) => {}
            }
        }

        if ambiguous {
            None
        } else {
            let peer = best_peer_index.and_then(|peer_index| self.peers.get(peer_index));
            peer.or_else(|| select_paid_route_peer_for_ip(&self.paid_route_peers, destination))
        }
    }

    fn peer_allows_inbound_destination(
        &self,
        peer: &FipsMeshPeerRuntime,
        destination: IpAddr,
    ) -> bool {
        let paid_admission = peer
            .participant_pubkey
            .as_ref()
            .and_then(|participant| self.paid_route_admissions.get(participant));
        if paid_admission.is_some_and(|admission| !admission.allow_routing) {
            return false;
        }
        if self.local_routes.is_empty() {
            return true;
        }
        let Some(local_route) = self.select_local_route_for_ip(destination) else {
            return false;
        };
        if paid_admission.is_some() {
            return local_route.is_default_route();
        }
        true
    }

    fn select_local_route_for_ip(&self, destination: IpAddr) -> Option<IpRoute> {
        self.local_routes
            .iter()
            .copied()
            .filter(|route| route.matches(destination))
            .max_by_key(|route| route.prefix_len)
    }
}

fn normalize_paid_route_admissions(
    admissions: Vec<FipsPaidRouteAdmission>,
) -> HashMap<[u8; 32], FipsPaidRouteAdmission> {
    let mut by_participant = HashMap::new();
    for mut admission in admissions {
        let Some(participant_pubkey) = parse_nostr_pubkey_bytes(&admission.participant_pubkey)
        else {
            continue;
        };
        admission.participant_pubkey = hex::encode(participant_pubkey);
        let replace = by_participant
            .get(&participant_pubkey)
            .is_none_or(|existing| paid_route_admission_preferred(&admission, existing));
        if replace {
            by_participant.insert(participant_pubkey, admission);
        }
    }
    by_participant
}

fn paid_route_admission_preferred(
    candidate: &FipsPaidRouteAdmission,
    existing: &FipsPaidRouteAdmission,
) -> bool {
    match (candidate.allow_routing, existing.allow_routing) {
        (true, false) => true,
        (false, true) => false,
        _ => candidate.updated_at_unix > existing.updated_at_unix,
    }
}

fn paid_route_peers_from_admissions(
    admissions: &HashMap<[u8; 32], FipsPaidRouteAdmission>,
) -> Vec<FipsMeshPeerRuntime> {
    let mut peers = admissions
        .values()
        .filter_map(|admission| {
            let routes = admission
                .allowed_ips
                .iter()
                .filter_map(|route| IpRoute::parse(route))
                .collect::<Vec<_>>();
            if routes.is_empty() {
                return None;
            }
            let participant_pubkey = parse_nostr_pubkey_bytes(&admission.participant_pubkey)?;
            let endpoint_node_addr = endpoint_node_addr_from_pubkey_bytes(participant_pubkey);
            let endpoint_npub = npub_for_pubkey_bytes(&participant_pubkey).ok()?;
            Some(FipsMeshPeerRuntime {
                participant_pubkey: Some(participant_pubkey),
                participant_pubkey_hex: hex::encode(participant_pubkey),
                endpoint_npub: Some(endpoint_npub),
                endpoint_pubkey: Some(participant_pubkey),
                endpoint_node_addr: Some(endpoint_node_addr),
                routes,
            })
        })
        .collect::<Vec<_>>();
    peers.sort_by(|left, right| {
        left.participant_pubkey_hex
            .cmp(&right.participant_pubkey_hex)
    });
    peers.dedup_by(|left, right| same_participant(left, right));
    peers
}

fn select_paid_route_peer_for_ip(
    peers: &[FipsMeshPeerRuntime],
    destination: IpAddr,
) -> Option<&FipsMeshPeerRuntime> {
    let mut best_peer = None;
    let mut best_prefix = None;
    let mut ambiguous = false;

    for peer in peers {
        for route in &peer.routes {
            if !route.matches(destination) {
                continue;
            }
            match best_prefix {
                None => {
                    best_peer = Some(peer);
                    best_prefix = Some(route.prefix_len);
                    ambiguous = false;
                }
                Some(prefix) if route.prefix_len > prefix => {
                    best_peer = Some(peer);
                    best_prefix = Some(route.prefix_len);
                    ambiguous = false;
                }
                Some(prefix)
                    if route.prefix_len == prefix
                        && best_peer.is_some_and(|best| !same_participant(best, peer)) =>
                {
                    ambiguous = true;
                }
                Some(_) => {}
            }
        }
    }

    if ambiguous { None } else { best_peer }
}

fn participant_peer_index(peers: &[FipsMeshPeerRuntime]) -> HashMap<[u8; 32], usize> {
    let mut index = HashMap::new();
    for (peer_index, peer) in peers.iter().enumerate() {
        if let Some(participant_pubkey) = peer.participant_pubkey {
            index.entry(participant_pubkey).or_insert(peer_index);
        }
    }
    index
}

fn endpoint_peer_indexes(
    peers: &[FipsMeshPeerRuntime],
) -> (HashMap<[u8; 32], usize>, HashMap<[u8; 16], usize>) {
    let mut pubkeys = HashMap::new();
    let mut node_addrs = HashMap::new();
    for (peer_index, peer) in peers.iter().enumerate() {
        if let Some(endpoint_pubkey) = peer.endpoint_pubkey {
            pubkeys.entry(endpoint_pubkey).or_insert(peer_index);
        }
        if let Some(endpoint_node_addr) = peer.endpoint_node_addr {
            node_addrs.entry(endpoint_node_addr).or_insert(peer_index);
        }
    }
    (pubkeys, node_addrs)
}

fn exact_route_peer_index(peers: &[FipsMeshPeerRuntime]) -> HashMap<IpAddr, ExactRouteMatch> {
    let mut index = HashMap::new();
    for (peer_index, peer) in peers.iter().enumerate() {
        for route in &peer.routes {
            let Some(exact_ip) = route.exact_ip() else {
                continue;
            };
            index
                .entry(exact_ip)
                .and_modify(|entry| {
                    if let ExactRouteMatch::Peer(existing_index) = *entry
                        && same_participant(&peers[existing_index], peer)
                    {
                        return;
                    }
                    *entry = ExactRouteMatch::Ambiguous;
                })
                .or_insert(ExactRouteMatch::Peer(peer_index));
        }
    }
    index
}

fn prefix_route_peer_indexes(
    peers: &[FipsMeshPeerRuntime],
) -> (Vec<IndexedIpRoute>, Vec<IndexedIpRoute>) {
    let mut v4 = Vec::new();
    let mut v6 = Vec::new();
    for (peer_index, peer) in peers.iter().enumerate() {
        for &route in &peer.routes {
            if route.exact_ip().is_some() {
                continue;
            }
            let indexed = IndexedIpRoute { peer_index, route };
            match route.network {
                IpAddr::V4(_) => v4.push(indexed),
                IpAddr::V6(_) => v6.push(indexed),
            }
        }
    }

    sort_prefix_route_peer_index(&mut v4);
    sort_prefix_route_peer_index(&mut v6);
    (v4, v6)
}

fn sort_prefix_route_peer_index(routes: &mut [IndexedIpRoute]) {
    routes.sort_by(|left, right| {
        right
            .route
            .prefix_len
            .cmp(&left.route.prefix_len)
            .then_with(|| left.peer_index.cmp(&right.peer_index))
    });
}

fn endpoint_node_addr_from_pubkey_bytes(pubkey: [u8; 32]) -> [u8; 16] {
    let digest = Sha256::digest(pubkey);
    let mut node_addr = [0u8; 16];
    node_addr.copy_from_slice(&digest[..16]);
    node_addr
}

fn runtime_participant_pubkey(value: &str) -> (Option<[u8; 32]>, String) {
    if let Some(pubkey) = parse_nostr_pubkey_bytes(value) {
        return (Some(pubkey), hex::encode(pubkey));
    }
    (None, value.trim().to_string())
}

fn same_participant(left: &FipsMeshPeerRuntime, right: &FipsMeshPeerRuntime) -> bool {
    match (left.participant_pubkey, right.participant_pubkey) {
        (Some(left), Some(right)) => left == right,
        _ => left.participant_pubkey_hex == right.participant_pubkey_hex,
    }
}

fn parse_nostr_pubkey_bytes(value: &str) -> Option<[u8; 32]> {
    PublicKey::parse(value.trim())
        .ok()
        .map(|pubkey| *pubkey.as_bytes())
}

fn npub_for_pubkey_hex(pubkey_hex: &str) -> Result<String> {
    PublicKey::from_hex(pubkey_hex)
        .context("invalid endpoint public key")?
        .to_bech32()
        .context("failed to encode endpoint npub")
}

fn npub_for_pubkey_bytes(pubkey: &[u8; 32]) -> Result<String> {
    PublicKey::from_byte_array(*pubkey)
        .to_bech32()
        .context("failed to encode endpoint npub")
}

impl IpRoute {
    fn parse(value: &str) -> Option<Self> {
        let (addr, prefix_len) = value.trim().split_once('/')?;
        let network = addr.trim().parse::<IpAddr>().ok()?;
        let prefix_len = prefix_len.trim().parse::<u8>().ok()?;

        match network {
            IpAddr::V4(ip) if prefix_len <= 32 => Some(Self {
                network: IpAddr::V4(mask_ipv4(ip, prefix_len)),
                prefix_len,
            }),
            IpAddr::V6(ip) if prefix_len <= 128 => Some(Self {
                network: IpAddr::V6(mask_ipv6(ip, prefix_len)),
                prefix_len,
            }),
            _ => None,
        }
    }

    fn matches(self, ip: IpAddr) -> bool {
        match (self.network, ip) {
            (IpAddr::V4(network), IpAddr::V4(ip)) => mask_ipv4(ip, self.prefix_len) == network,
            (IpAddr::V6(network), IpAddr::V6(ip)) => mask_ipv6(ip, self.prefix_len) == network,
            _ => false,
        }
    }

    fn exact_ip(self) -> Option<IpAddr> {
        match self.network {
            IpAddr::V4(ip) if self.prefix_len == 32 => Some(IpAddr::V4(ip)),
            IpAddr::V6(ip) if self.prefix_len == 128 => Some(IpAddr::V6(ip)),
            _ => None,
        }
    }

    fn is_default_route(self) -> bool {
        matches!(
            (self.network, self.prefix_len),
            (IpAddr::V4(ip), 0) if ip == Ipv4Addr::UNSPECIFIED
        ) || matches!(
            (self.network, self.prefix_len),
            (IpAddr::V6(ip), 0) if ip == Ipv6Addr::UNSPECIFIED
        )
    }
}

pub fn packet_destination(packet: &[u8]) -> Option<IpAddr> {
    match packet.first()? >> 4 {
        4 => ipv4_packet_addr(packet, 16),
        6 => ipv6_packet_addr(packet, 24),
        _ => None,
    }
}

fn packet_source(packet: &[u8]) -> Option<IpAddr> {
    match packet.first()? >> 4 {
        4 => ipv4_packet_addr(packet, 12),
        6 => ipv6_packet_addr(packet, 8),
        _ => None,
    }
}

fn ipv4_packet_addr(packet: &[u8], offset: usize) -> Option<IpAddr> {
    if packet.len() < 20 || offset + 4 > packet.len() {
        return None;
    }
    let ihl = packet[0] & 0x0f;
    if ihl < 5 || packet.len() < usize::from(ihl) * 4 {
        return None;
    }

    Some(IpAddr::V4(Ipv4Addr::new(
        packet[offset],
        packet[offset + 1],
        packet[offset + 2],
        packet[offset + 3],
    )))
}

fn ipv6_packet_addr(packet: &[u8], offset: usize) -> Option<IpAddr> {
    if packet.len() < 40 || offset + 16 > packet.len() {
        return None;
    }

    let mut octets = [0_u8; 16];
    octets.copy_from_slice(&packet[offset..offset + 16]);
    Some(IpAddr::V6(Ipv6Addr::from(octets)))
}

fn mask_ipv4(ip: Ipv4Addr, bits: u8) -> Ipv4Addr {
    let mask = if bits == 0 {
        0
    } else {
        u32::MAX << (32 - bits)
    };
    Ipv4Addr::from(u32::from(ip) & mask)
}

fn mask_ipv6(ip: Ipv6Addr, bits: u8) -> Ipv6Addr {
    let mask = if bits == 0 {
        0
    } else {
        u128::MAX << (128 - bits)
    };
    Ipv6Addr::from(u128::from_be_bytes(ip.octets()) & mask)
}

#[cfg(test)]

include!("fips_mesh/tests.rs");
