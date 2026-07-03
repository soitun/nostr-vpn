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
pub struct AcceptedFipsPacket<'a, B = Vec<u8>> {
    pub source_pubkey: &'a str,
    pub source_pubkey_bytes: Option<&'a [u8; 32]>,
    pub bytes: B,
}

#[derive(Debug, PartialEq, Eq)]
pub struct AcceptedFipsPacketSourceRun<'a> {
    pub source_pubkey: &'a str,
    pub source_pubkey_bytes: Option<&'a [u8; 32]>,
    pub endpoint_bytes: usize,
    accepted_packets: usize,
}

impl AcceptedFipsPacketSourceRun<'_> {
    pub fn len(&self) -> usize {
        self.accepted_packets
    }

    pub fn is_empty(&self) -> bool {
        self.accepted_packets == 0
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FipsEndpointSourceAdmitter<'a> {
    runtime: &'a FipsMeshRuntime,
    peer: &'a FipsMeshPeerRuntime,
}

#[derive(Debug, Clone)]
pub struct FipsMeshRuntime {
    peers: Vec<FipsMeshPeerRuntime>,
    local_routes: Vec<IpRoute>,
    paid_route_admissions: HashMap<[u8; 32], FipsPaidRouteAdmission>,
    paid_route_peers: Vec<FipsMeshPeerRuntime>,
    paid_route_routing_peers: Vec<FipsMeshPeerRuntime>,
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
        let paid_route_peers = paid_route_peers_from_admissions(&paid_route_admissions, false);
        let paid_route_routing_peers =
            paid_route_peers_from_admissions(&paid_route_admissions, true);

        Self {
            peers,
            local_routes,
            paid_route_admissions,
            paid_route_peers,
            paid_route_routing_peers,
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
        self.paid_route_peers =
            paid_route_peers_from_admissions(&self.paid_route_admissions, false);
        self.paid_route_routing_peers =
            paid_route_peers_from_admissions(&self.paid_route_admissions, true);
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

        routed_fips_peer(peer)
    }

    pub fn route_outbound_destination_peer<'a>(
        &'a self,
        destination: IpAddr,
    ) -> Option<RoutedFipsPeer<'a>> {
        let peer = self.select_peer_for_ip(destination)?;

        routed_fips_peer(peer)
    }

    pub fn route_outbound_packet_with_peer<'a>(
        &'a self,
        packet: &[u8],
    ) -> Option<RoutedFipsPacket<'a>> {
        let peer = self.route_outbound_peer(packet)?;

        routed_fips_packet(peer, packet.to_vec())
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

        routed_fips_packet(peer, packet)
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

    pub fn receive_endpoint_data_owned_with_source_node_addr<'a, B>(
        &'a self,
        source_node_addr: &[u8; 16],
        data: B,
    ) -> Option<AcceptedFipsPacket<'a, B>>
    where
        B: AsRef<[u8]>,
    {
        self.endpoint_source_admitter(source_node_addr)?
            .receive_owned(data)
    }

    pub fn endpoint_source_admitter<'a>(
        &'a self,
        source_node_addr: &[u8; 16],
    ) -> Option<FipsEndpointSourceAdmitter<'a>> {
        let peer = self.peer_for_endpoint_node_addr(source_node_addr)?;
        Some(FipsEndpointSourceAdmitter {
            runtime: self,
            peer,
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
        self.endpoint_source_admitter(source_node_addr)?.admit(data)
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
        self.peer_for_endpoint_pubkey(&source_pubkey)
            .map(|peer| peer.participant_pubkey_hex.clone())
    }

    pub fn participant_for_endpoint_node_addr(
        &self,
        endpoint_node_addr: &[u8; 16],
    ) -> Option<String> {
        self.peer_for_endpoint_node_addr(endpoint_node_addr)
            .map(|peer| peer.participant_pubkey_hex.clone())
    }

    pub fn participant_pubkey_bytes_for_endpoint_node_addr(
        &self,
        endpoint_node_addr: &[u8; 16],
    ) -> Option<[u8; 32]> {
        self.peer_for_endpoint_node_addr(endpoint_node_addr)
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
        self.peer_for_participant_pubkey_bytes(participant_pubkey)
            .and_then(|peer| peer.endpoint_node_addr)
    }

    pub fn peer_pubkeys(&self) -> Vec<String> {
        let mut pubkeys = Vec::new();
        for peer in self.peers.iter().chain(self.paid_route_peers.iter()) {
            if !pubkeys
                .iter()
                .any(|pubkey| pubkey == &peer.participant_pubkey_hex)
            {
                pubkeys.push(peer.participant_pubkey_hex.clone());
            }
        }
        pubkeys
    }

    fn peer_for_endpoint_pubkey(&self, endpoint_pubkey: &[u8; 32]) -> Option<&FipsMeshPeerRuntime> {
        self.endpoint_pubkey_peer_index
            .get(endpoint_pubkey)
            .and_then(|peer_index| self.peers.get(*peer_index))
            .or_else(|| {
                self.paid_route_peers
                    .iter()
                    .find(|peer| peer.endpoint_pubkey.as_ref() == Some(endpoint_pubkey))
            })
    }

    fn peer_for_endpoint_node_addr(
        &self,
        endpoint_node_addr: &[u8; 16],
    ) -> Option<&FipsMeshPeerRuntime> {
        self.endpoint_node_addr_peer_index
            .get(endpoint_node_addr)
            .and_then(|peer_index| self.peers.get(*peer_index))
            .or_else(|| {
                self.paid_route_peers
                    .iter()
                    .find(|peer| peer.endpoint_node_addr.as_ref() == Some(endpoint_node_addr))
            })
    }

    fn peer_for_participant_pubkey_bytes(
        &self,
        participant_pubkey: &[u8; 32],
    ) -> Option<&FipsMeshPeerRuntime> {
        self.participant_peer_index
            .get(participant_pubkey)
            .and_then(|peer_index| self.peers.get(*peer_index))
            .or_else(|| {
                self.paid_route_peers
                    .iter()
                    .find(|peer| peer.participant_pubkey.as_ref() == Some(participant_pubkey))
            })
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
            peer.or_else(|| {
                select_paid_route_peer_for_ip(&self.paid_route_routing_peers, destination)
            })
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

impl<'a> FipsEndpointSourceAdmitter<'a> {
    pub fn source_pubkey(&self) -> &'a str {
        &self.peer.participant_pubkey_hex
    }

    pub fn source_pubkey_bytes(&self) -> Option<&'a [u8; 32]> {
        self.peer.participant_pubkey.as_ref()
    }

    pub fn admit_packet(&self, data: &[u8]) -> bool {
        self.admit(data).is_some()
    }

    pub fn receive_owned<B>(&self, data: B) -> Option<AcceptedFipsPacket<'a, B>>
    where
        B: AsRef<[u8]>,
    {
        self.admit(data.as_ref())?;
        Some(AcceptedFipsPacket {
            source_pubkey: &self.peer.participant_pubkey_hex,
            source_pubkey_bytes: self.peer.participant_pubkey.as_ref(),
            bytes: data,
        })
    }

    pub fn receive_owned_source_run_into<I, B, F>(
        &self,
        packets: I,
        mut accept: F,
    ) -> Option<AcceptedFipsPacketSourceRun<'a>>
    where
        I: IntoIterator<Item = B>,
        B: AsRef<[u8]>,
        F: FnMut(B),
    {
        let mut accepted_packets = 0usize;
        let mut endpoint_bytes = 0usize;
        for packet in packets {
            let bytes = packet.as_ref();
            if self.admit(bytes).is_some() {
                endpoint_bytes = endpoint_bytes.saturating_add(bytes.len());
                accepted_packets = accepted_packets.saturating_add(1);
                accept(packet);
            }
        }
        if accepted_packets == 0 {
            return None;
        }
        Some(AcceptedFipsPacketSourceRun {
            source_pubkey: &self.peer.participant_pubkey_hex,
            source_pubkey_bytes: self.peer.participant_pubkey.as_ref(),
            endpoint_bytes,
            accepted_packets,
        })
    }

    fn admit(&self, data: &[u8]) -> Option<&'a FipsMeshPeerRuntime> {
        let packet_source = packet_source(data)?;
        if !self.peer_allows_inbound_source(packet_source) {
            return None;
        }
        let packet_destination = packet_destination(data)?;
        if !self
            .runtime
            .peer_allows_inbound_destination(self.peer, packet_destination)
        {
            return None;
        }
        Some(self.peer)
    }

    fn peer_allows_inbound_source(&self, source: IpAddr) -> bool {
        self.peer.routes.iter().any(|route| route.matches(source))
    }
}

include!("fips_mesh/route_helpers.rs");

#[cfg(test)]

include!("fips_mesh/tests.rs");
