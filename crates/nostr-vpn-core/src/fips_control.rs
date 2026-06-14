use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use nostr_sdk::prelude::{Event, EventBuilder, Keys, Kind, PublicKey, Tag, Timestamp};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use crate::join_requests::MeshJoinRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkRoster {
    #[serde(default)]
    pub network_name: String,
    pub participants: Vec<String>,
    #[serde(default)]
    pub admins: Vec<String>,
    #[serde(default)]
    pub aliases: HashMap<String, String>,
    #[serde(default)]
    pub signed_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedRoster {
    pub event: Event,
}

const SIGNED_ROSTER_KIND: u16 = 30_388;
const SIGNED_ROSTER_VERSION: &str = "1";
const SIGNED_ROSTER_APP: &str = "nostr-vpn/shared-roster";

impl SignedRoster {
    pub fn sign(network_id: impl Into<String>, roster: NetworkRoster, keys: &Keys) -> Result<Self> {
        let event = EventBuilder::new(Kind::Custom(SIGNED_ROSTER_KIND), "")
            .tags(signed_roster_tags(&network_id.into(), &roster)?)
            .custom_created_at(Timestamp::from(roster.signed_at))
            .sign_with_keys(keys)
            .map_err(|error| anyhow!("failed to sign roster event: {error}"))?;
        let signed = Self { event };
        signed.verify()?;
        Ok(signed)
    }

    pub fn from_event(event: Event) -> Result<Self> {
        let signed = Self { event };
        signed.verify()?;
        Ok(signed)
    }

    pub fn verify(&self) -> Result<()> {
        if u16::from(self.event.kind) != SIGNED_ROSTER_KIND {
            return Err(anyhow!(
                "unexpected roster event kind {}",
                u16::from(self.event.kind)
            ));
        }
        if !self.event.content.is_empty() {
            return Err(anyhow!("signed roster event content must be empty"));
        }
        self.event
            .verify()
            .map_err(|error| anyhow!("invalid roster event signature: {error}"))?;
        let _ = self.to_network_roster()?;
        Ok(())
    }

    pub fn signer_pubkey_hex(&self) -> Result<String> {
        Ok(self.event.pubkey.to_hex())
    }

    pub fn network_id(&self) -> Result<String> {
        Ok(self.to_network_roster()?.0)
    }

    pub fn roster(&self) -> Result<NetworkRoster> {
        Ok(self.to_network_roster()?.1)
    }

    pub fn signed_at(&self) -> u64 {
        self.event.created_at.as_secs()
    }

    pub fn content_hash(&self) -> String {
        self.event.id.to_hex()
    }

    pub fn artifact_hash(&self) -> String {
        self.event.id.to_hex()
    }

    fn to_network_roster(&self) -> Result<(String, NetworkRoster)> {
        signed_roster_from_tags(self.event.tags.as_slice(), self.event.created_at.as_secs())
    }
}

fn signed_roster_tags(network_id: &str, roster: &NetworkRoster) -> Result<Vec<Tag>> {
    let mut tags = vec![
        Tag::identifier(network_id.trim().to_string()),
        roster_tag(&["app", SIGNED_ROSTER_APP])?,
        roster_tag(&["v", SIGNED_ROSTER_VERSION])?,
    ];

    if !roster.network_name.trim().is_empty() {
        tags.push(roster_tag(&["name", roster.network_name.trim()])?);
    }

    let mut participants = normalize_roster_pubkeys(&roster.participants, "participant")?;
    participants.sort();
    participants.dedup();
    for participant in participants {
        tags.push(roster_tag(&["member", &participant])?);
    }

    let mut admins = normalize_roster_pubkeys(&roster.admins, "admin")?;
    admins.sort();
    admins.dedup();
    for admin in admins {
        tags.push(roster_tag(&["admin", &admin])?);
    }

    let mut aliases = roster
        .aliases
        .iter()
        .map(|(pubkey, alias)| {
            Ok((
                normalize_roster_pubkey(pubkey, "alias")?,
                alias.trim().to_string(),
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    aliases.sort_by(|left, right| left.0.cmp(&right.0));
    aliases.dedup_by(|left, right| left.0 == right.0);
    for (pubkey, alias) in aliases {
        if !alias.is_empty() {
            tags.push(roster_tag(&["alias", &pubkey, &alias])?);
        }
    }

    Ok(tags)
}

fn signed_roster_from_tags(tags: &[Tag], signed_at: u64) -> Result<(String, NetworkRoster)> {
    let mut app_ok = false;
    let mut version_ok = false;
    let mut network_id = None;
    let mut network_name = String::new();
    let mut participants = Vec::new();
    let mut admins = Vec::new();
    let mut aliases = HashMap::new();

    for tag in tags {
        let parts = tag.as_slice();
        let Some(kind) = parts.first().map(String::as_str) else {
            continue;
        };
        match kind {
            "d" => network_id = parts.get(1).map(|value| value.trim().to_string()),
            "app" => app_ok |= parts.get(1).is_some_and(|value| value == SIGNED_ROSTER_APP),
            "v" => {
                version_ok |= parts
                    .get(1)
                    .is_some_and(|value| value == SIGNED_ROSTER_VERSION)
            }
            "name" => {
                if let Some(value) = parts.get(1) {
                    network_name = value.trim().to_string();
                }
            }
            "member" => {
                if let Some(value) = parts.get(1) {
                    participants.push(normalize_roster_pubkey(value, "participant")?);
                }
            }
            "admin" => {
                if let Some(value) = parts.get(1) {
                    admins.push(normalize_roster_pubkey(value, "admin")?);
                }
            }
            "alias" => {
                if let (Some(pubkey), Some(alias)) = (parts.get(1), parts.get(2)) {
                    let pubkey = normalize_roster_pubkey(pubkey, "alias")?;
                    let alias = alias.trim();
                    if !alias.is_empty() {
                        aliases.insert(pubkey, alias.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    if !app_ok {
        return Err(anyhow!("signed roster event is missing app tag"));
    }
    if !version_ok {
        return Err(anyhow!("signed roster event has unsupported version"));
    }

    participants.sort();
    participants.dedup();
    admins.sort();
    admins.dedup();

    let network_id = network_id
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("signed roster event is missing network identifier"))?;
    Ok((
        network_id,
        NetworkRoster {
            network_name,
            participants,
            admins,
            aliases,
            signed_at,
        },
    ))
}

fn roster_tag(parts: &[&str]) -> Result<Tag> {
    Tag::parse(parts.iter().copied()).map_err(|error| anyhow!("invalid roster event tag: {error}"))
}

fn normalize_roster_pubkeys(values: &[String], role: &str) -> Result<Vec<String>> {
    values
        .iter()
        .map(|value| normalize_roster_pubkey(value, role))
        .collect()
}

fn normalize_roster_pubkey(value: &str, role: &str) -> Result<String> {
    PublicKey::parse(value.trim())
        .map(|pubkey| pubkey.to_hex())
        .map_err(|error| anyhow!("invalid roster {role} pubkey: {error}"))
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerCapabilities {
    #[serde(default)]
    pub advertised_routes: Vec<String>,
    #[serde(default)]
    pub endpoint_hints: Vec<PeerEndpointHint>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub direct_fmp_endpoint_data: bool,
    #[serde(default)]
    pub signed_at: u64,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerEndpointHint {
    #[serde(default = "default_peer_endpoint_hint_transport")]
    pub transport: String,
    pub addr: String,
}

impl PeerEndpointHint {
    pub fn udp(addr: impl Into<String>) -> Self {
        Self {
            transport: default_peer_endpoint_hint_transport(),
            addr: addr.into(),
        }
    }
}

fn default_peer_endpoint_hint_transport() -> String {
    "udp".to_string()
}

const FIPS_CONTROL_MAGIC: &[u8] = b"NVPN-FIPS-CTRL\0";
const FIPS_CONTROL_VERSION: u8 = 1;
pub const FIPS_CONTROL_DIRECT_FRAME_LIMIT: usize = 1100;
pub const FIPS_CONTROL_FRAGMENT_CHUNK_LEN: usize = 700;
pub const FIPS_CONTROL_FRAGMENT_TTL_SECS: u64 = 120;
pub const FIPS_CONTROL_MAX_FRAGMENTS: u16 = 128;
pub const FIPS_CONTROL_MAX_REASSEMBLED_LEN: usize = 128 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FipsControlFrame {
    Ping {
        network_id: String,
        sent_at: u64,
    },
    Pong {
        network_id: String,
        sent_at: u64,
        replied_at: u64,
    },
    JoinRequest {
        requested_at: u64,
        request: MeshJoinRequest,
    },
    Roster {
        network_id: String,
        roster: NetworkRoster,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signed_roster: Option<Box<SignedRoster>>,
    },
    Capabilities {
        network_id: String,
        capabilities: PeerCapabilities,
    },
    Fragment {
        id: String,
        index: u16,
        total: u16,
        data: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FipsControlEnvelope {
    v: u8,
    frame: FipsControlFrame,
}

pub fn encode_fips_control_frame(frame: &FipsControlFrame) -> Result<Vec<u8>> {
    let envelope = FipsControlEnvelope {
        v: FIPS_CONTROL_VERSION,
        frame: frame.clone(),
    };
    let mut out = Vec::from(FIPS_CONTROL_MAGIC);
    out.extend_from_slice(
        &serde_json::to_vec(&envelope).context("failed to encode FIPS control frame")?,
    );
    Ok(out)
}

pub fn encode_fips_control_messages(frame: &FipsControlFrame) -> Result<Vec<Vec<u8>>> {
    let encoded = encode_fips_control_frame(frame)?;
    if encoded.len() <= FIPS_CONTROL_DIRECT_FRAME_LIMIT {
        return Ok(vec![encoded]);
    }

    let total = encoded.len().div_ceil(FIPS_CONTROL_FRAGMENT_CHUNK_LEN);
    let total = u16::try_from(total).context("FIPS control frame has too many fragments")?;
    let id = fips_control_fragment_id(&encoded);
    encoded
        .chunks(FIPS_CONTROL_FRAGMENT_CHUNK_LEN)
        .enumerate()
        .map(|(index, chunk)| {
            let fragment = FipsControlFrame::Fragment {
                id: id.clone(),
                index: u16::try_from(index).context("FIPS control fragment index overflow")?,
                total,
                data: URL_SAFE_NO_PAD.encode(chunk),
            };
            encode_fips_control_frame(&fragment)
        })
        .collect()
}

pub fn decode_fips_control_frame(data: &[u8]) -> Result<Option<FipsControlFrame>> {
    let Some(payload) = data.strip_prefix(FIPS_CONTROL_MAGIC) else {
        return Ok(None);
    };
    let envelope: FipsControlEnvelope = match serde_json::from_slice(payload) {
        Ok(envelope) => envelope,
        Err(_) => return Ok(None),
    };
    if envelope.v != FIPS_CONTROL_VERSION {
        return Ok(None);
    }
    Ok(Some(envelope.frame))
}

#[derive(Debug, Clone, Default)]
pub struct FipsControlFragmentBuffer {
    entries: HashMap<ControlFragmentKey, PendingControlFragment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ControlFragmentKey {
    source_key: Vec<u8>,
    id: String,
}

#[derive(Debug, Clone)]
struct PendingControlFragment {
    total: u16,
    received_at: u64,
    chunks: Vec<Option<Vec<u8>>>,
    received_len: usize,
}

impl FipsControlFragmentBuffer {
    pub fn decode(
        &mut self,
        source_key: impl AsRef<[u8]>,
        data: &[u8],
        now: u64,
    ) -> Result<Option<FipsControlFrame>> {
        let source_key = source_key.as_ref();
        let Some(frame) = decode_fips_control_frame(data)? else {
            return Ok(None);
        };
        let FipsControlFrame::Fragment {
            id,
            index,
            total,
            data,
        } = frame
        else {
            return Ok(Some(frame));
        };

        let Some(reassembled) = self.push(source_key, id, index, total, data, now)? else {
            return Ok(None);
        };
        decode_fips_control_frame(&reassembled)
    }

    pub fn push(
        &mut self,
        source_key: impl AsRef<[u8]>,
        id: String,
        index: u16,
        total: u16,
        data: String,
        now: u64,
    ) -> Result<Option<Vec<u8>>> {
        let source_key = source_key.as_ref();
        if total == 0 || total > FIPS_CONTROL_MAX_FRAGMENTS || index >= total {
            return Ok(None);
        }

        self.entries.retain(|_, entry| {
            now.saturating_sub(entry.received_at) <= FIPS_CONTROL_FRAGMENT_TTL_SECS
        });

        let Ok(decoded) = URL_SAFE_NO_PAD.decode(data.as_bytes()) else {
            return Ok(None);
        };
        if decoded.len() > FIPS_CONTROL_FRAGMENT_CHUNK_LEN {
            return Ok(None);
        }

        let key = ControlFragmentKey {
            source_key: source_key.to_vec(),
            id,
        };
        let entry = self
            .entries
            .entry(key.clone())
            .or_insert_with(|| PendingControlFragment {
                total,
                received_at: now,
                chunks: vec![None; usize::from(total)],
                received_len: 0,
            });
        if entry.total != total {
            *entry = PendingControlFragment {
                total,
                received_at: now,
                chunks: vec![None; usize::from(total)],
                received_len: 0,
            };
        }
        entry.received_at = now;

        let slot = &mut entry.chunks[usize::from(index)];
        if let Some(existing) = slot.as_ref() {
            entry.received_len = entry.received_len.saturating_sub(existing.len());
        }
        entry.received_len += decoded.len();
        if entry.received_len > FIPS_CONTROL_MAX_REASSEMBLED_LEN {
            self.entries.remove(&key);
            return Ok(None);
        }
        *slot = Some(decoded);

        if !entry.chunks.iter().all(|chunk| chunk.is_some()) {
            return Ok(None);
        }

        let entry = self
            .entries
            .remove(&key)
            .expect("complete fragment entry should exist");
        let mut reassembled = Vec::with_capacity(entry.received_len);
        for chunk in entry.chunks.into_iter().flatten() {
            reassembled.extend_from_slice(&chunk);
        }
        Ok(Some(reassembled))
    }
}

pub fn peer_endpoint_hint_addr(hint: &PeerEndpointHint) -> Option<String> {
    if !hint.transport.trim().eq_ignore_ascii_case("udp") {
        return None;
    }
    let trimmed = hint.addr.trim();
    if let Ok(parsed) = trimmed.parse::<SocketAddr>() {
        if parsed.port() == 0 || endpoint_hint_ip_is_unusable(parsed.ip()) {
            return None;
        }
        return Some(parsed.to_string());
    }

    let (host, port) = trimmed.rsplit_once(':')?;
    let host = host.trim();
    let port = port.trim().parse::<u16>().ok()?;
    if host.is_empty() || port == 0 || host.eq_ignore_ascii_case("localhost") {
        return None;
    }
    if host_looks_like_nostr_pubkey(host) {
        return None;
    }
    if host.contains(':') {
        return None;
    }
    if let Ok(ip) = host.parse::<IpAddr>()
        && endpoint_hint_ip_is_unusable(ip)
    {
        return None;
    }
    if host.parse::<IpAddr>().is_err() && !endpoint_hint_host_is_valid(host) {
        return None;
    }
    Some(format!("{host}:{port}"))
}

fn endpoint_hint_host_is_valid(host: &str) -> bool {
    let host = host.trim_end_matches('.');
    if host.is_empty() || host.len() > 253 {
        return false;
    }
    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    })
}

fn endpoint_hint_ip_is_unusable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_multicast()
                || ipv4_is_cgnat(ip)
                || ipv4_is_documentation(ip)
        }
        IpAddr::V6(ip) => {
            ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_unicast_link_local()
                || ip.is_multicast()
        }
    }
}

fn ipv4_is_cgnat(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 100 && (64..=127).contains(&octets[1])
}

fn ipv4_is_documentation(ip: Ipv4Addr) -> bool {
    matches!(
        ip.octets(),
        [192, 0, 2, _] | [198, 51, 100, _] | [203, 0, 113, _]
    )
}

fn host_looks_like_nostr_pubkey(host: &str) -> bool {
    let host = host.trim().to_ascii_lowercase();
    host.len() >= 60 && host.starts_with("npub1")
}

fn fips_control_fragment_id(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

pub fn roster_control_frame(
    network_id: impl Into<String>,
    roster: NetworkRoster,
) -> FipsControlFrame {
    FipsControlFrame::Roster {
        network_id: network_id.into(),
        roster,
        signed_roster: None,
    }
}

pub fn signed_roster_control_frame(signed_roster: SignedRoster) -> FipsControlFrame {
    let network_id = signed_roster.network_id().unwrap_or_default();
    let roster = signed_roster.roster().unwrap_or_else(|_| NetworkRoster {
        network_name: String::new(),
        participants: Vec::new(),
        admins: Vec::new(),
        aliases: HashMap::new(),
        signed_at: signed_roster.signed_at(),
    });
    FipsControlFrame::Roster {
        network_id,
        roster,
        signed_roster: Some(Box::new(signed_roster)),
    }
}

pub fn peer_capabilities_control_frame(
    network_id: impl Into<String>,
    capabilities: PeerCapabilities,
) -> FipsControlFrame {
    FipsControlFrame::Capabilities {
        network_id: network_id.into(),
        capabilities,
    }
}

pub fn network_roster_from_shared(
    network_name: String,
    participants: Vec<String>,
    admins: Vec<String>,
    aliases: HashMap<String, String>,
    signed_at: u64,
) -> NetworkRoster {
    NetworkRoster {
        network_name,
        participants,
        admins,
        aliases,
        signed_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_frame_roundtrips_with_magic_prefix() {
        let frame = FipsControlFrame::Ping {
            network_id: "mesh".to_string(),
            sent_at: 42,
        };

        let encoded = encode_fips_control_frame(&frame).expect("encode");
        assert!(encoded.starts_with(FIPS_CONTROL_MAGIC));

        let decoded = decode_fips_control_frame(&encoded)
            .expect("decode")
            .expect("control frame");
        assert_eq!(decoded, frame);
    }

    #[test]
    fn raw_packet_is_not_control() {
        let packet = [0x45, 0, 0, 20];

        assert!(
            decode_fips_control_frame(&packet)
                .expect("decode")
                .is_none()
        );
    }

    #[test]
    fn capabilities_frame_roundtrips() {
        let frame = FipsControlFrame::Capabilities {
            network_id: "mesh".to_string(),
            capabilities: PeerCapabilities {
                advertised_routes: vec!["0.0.0.0/0".to_string(), "::/0".to_string()],
                endpoint_hints: vec![PeerEndpointHint::udp("192.168.50.22:51820")],
                direct_fmp_endpoint_data: true,
                signed_at: 99,
            },
        };

        let encoded = encode_fips_control_frame(&frame).expect("encode");
        let decoded = decode_fips_control_frame(&encoded)
            .expect("decode")
            .expect("control frame");
        assert_eq!(decoded, frame);
    }

    #[test]
    fn old_capabilities_decode_with_empty_endpoint_hints() {
        let caps: PeerCapabilities =
            serde_json::from_str(r#"{"advertised_routes":["0.0.0.0/0"],"signed_at":99}"#)
                .expect("decode old capabilities");

        assert_eq!(caps.advertised_routes, vec!["0.0.0.0/0".to_string()]);
        assert!(caps.endpoint_hints.is_empty());
        assert!(!caps.direct_fmp_endpoint_data);
        assert_eq!(caps.signed_at, 99);
    }

    #[test]
    fn signed_roster_verifies_independent_of_alias_map_order() {
        let admin = Keys::generate();
        let alice = Keys::generate().public_key().to_hex();
        let bob = Keys::generate().public_key().to_hex();
        let mut aliases = HashMap::new();
        aliases.insert(bob.clone(), "bob".to_string());
        aliases.insert(alice.clone(), "alice".to_string());
        let roster = NetworkRoster {
            network_name: "Home".to_string(),
            participants: vec![bob.clone(), alice.clone()],
            admins: vec![admin.public_key().to_hex()],
            aliases,
            signed_at: 123,
        };

        let signed = SignedRoster::sign("mesh", roster, &admin).expect("sign roster");

        signed.verify().expect("verify signed roster");
        assert_eq!(
            signed.signer_pubkey_hex().unwrap(),
            admin.public_key().to_hex()
        );
        assert_eq!(signed.network_id().unwrap(), "mesh");
        assert_eq!(signed.roster().unwrap().network_name, "Home");
        assert_eq!(signed.content_hash().len(), 64);
        assert_eq!(signed.artifact_hash().len(), 64);
    }

    #[test]
    fn signed_roster_puts_roster_fields_in_tags() {
        let admin = Keys::generate();
        let member = Keys::generate().public_key().to_hex();
        let mut aliases = HashMap::new();
        aliases.insert(member.clone(), "phone".to_string());
        let roster = NetworkRoster {
            network_name: "Home".to_string(),
            participants: vec![member.clone()],
            admins: vec![admin.public_key().to_hex()],
            aliases,
            signed_at: 123,
        };

        let signed = SignedRoster::sign("mesh", roster, &admin).expect("sign roster");
        let tags = signed
            .event
            .tags
            .iter()
            .map(Tag::as_slice)
            .collect::<Vec<_>>();

        assert!(signed.event.content.is_empty());
        assert!(tags.contains(&vec!["d".to_string(), "mesh".to_string()].as_slice()));
        assert!(tags.contains(&vec!["member".to_string(), member].as_slice()));
        assert!(
            tags.iter()
                .any(|tag| tag.first().is_some_and(|tag| tag == "alias"))
        );
    }

    #[test]
    fn signed_roster_rejects_tampered_content() {
        let admin = Keys::generate();
        let member = Keys::generate().public_key().to_hex();
        let roster = NetworkRoster {
            network_name: "Home".to_string(),
            participants: vec![member],
            admins: vec![admin.public_key().to_hex()],
            aliases: HashMap::new(),
            signed_at: 123,
        };
        let signed = SignedRoster::sign("mesh", roster, &admin).expect("sign roster");
        let mut event = signed.event.clone();
        event.tags.push(roster_tag(&["name", "Office"]).unwrap());
        let signed = SignedRoster { event };

        assert!(signed.verify().is_err());
    }

    #[test]
    fn endpoint_hints_default_to_udp_transport() {
        let hint: PeerEndpointHint =
            serde_json::from_str(r#"{"addr":"192.168.50.22:51820"}"#).expect("decode hint");

        assert_eq!(hint.transport, "udp");
        assert_eq!(hint.addr, "192.168.50.22:51820");
    }

    #[test]
    fn peer_endpoint_hint_addr_accepts_lan_and_dns_udp_hints() {
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp("192.168.50.22:51820")),
            Some("192.168.50.22:51820".to_string())
        );
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp("peer.example.com:51820")),
            Some("peer.example.com:51820".to_string())
        );
    }

    #[test]
    fn peer_endpoint_hint_addr_rejects_unusable_hints() {
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint {
                transport: "tcp".to_string(),
                addr: "192.168.50.22:51820".to_string(),
            }),
            None
        );
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp("192.168.50.22")),
            None
        );
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp("127.0.0.1:51820")),
            None
        );
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp("100.120.94.10:51820")),
            None
        );
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp("198.51.100.10:51820")),
            None
        );
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp("0.0.0.0:51820")),
            None
        );
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp("localhost:51820")),
            None
        );
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp("not an endpoint:51820")),
            None
        );
        assert_eq!(
            peer_endpoint_hint_addr(&PeerEndpointHint::udp(format!(
                "{}:51820",
                "npub1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq"
            ))),
            None
        );
    }

    #[test]
    fn large_control_frame_fragments_under_direct_limit() {
        let roster = NetworkRoster {
            network_name: "Network 1".to_string(),
            participants: (0..12).map(|value| format!("{value:064x}")).collect(),
            admins: vec!["f".repeat(64)],
            aliases: (0..12)
                .map(|value| (format!("{value:064x}"), format!("node-{value}")))
                .collect(),
            signed_at: 123,
        };
        let frame = FipsControlFrame::Roster {
            network_id: "mesh".to_string(),
            roster,
            signed_roster: None,
        };

        let messages = encode_fips_control_messages(&frame).expect("fragment");

        assert!(messages.len() > 1);
        for message in messages {
            assert!(message.len() <= FIPS_CONTROL_DIRECT_FRAME_LIMIT);
            assert!(matches!(
                decode_fips_control_frame(&message).expect("decode"),
                Some(FipsControlFrame::Fragment { .. })
            ));
        }
    }

    #[test]
    fn fragment_buffer_decodes_fragmented_frame() {
        let roster = NetworkRoster {
            network_name: "Network 1".to_string(),
            participants: (0..12).map(|value| format!("{value:064x}")).collect(),
            admins: vec!["f".repeat(64)],
            aliases: (0..12)
                .map(|value| (format!("{value:064x}"), format!("node-{value}")))
                .collect(),
            signed_at: 123,
        };
        let frame = FipsControlFrame::Roster {
            network_id: "mesh".to_string(),
            roster,
            signed_roster: None,
        };
        let messages = encode_fips_control_messages(&frame).expect("fragment messages");
        let mut buffer = FipsControlFragmentBuffer::default();
        let mut decoded = None;

        for message in messages {
            decoded = buffer
                .decode("npub1source", &message, 1)
                .expect("decode with fragments")
                .or(decoded);
        }

        assert_eq!(decoded, Some(frame));
    }

    #[test]
    fn fragment_buffer_keys_sources_by_bytes() {
        let roster = NetworkRoster {
            network_name: "Network 1".to_string(),
            participants: (0..12).map(|value| format!("{value:064x}")).collect(),
            admins: vec!["f".repeat(64)],
            aliases: (0..12)
                .map(|value| (format!("{value:064x}"), format!("node-{value}")))
                .collect(),
            signed_at: 123,
        };
        let frame = FipsControlFrame::Roster {
            network_id: "mesh".to_string(),
            roster,
            signed_roster: None,
        };
        let messages = encode_fips_control_messages(&frame).expect("fragment messages");
        assert!(messages.len() > 1);
        let fragments: Vec<_> = messages
            .iter()
            .map(|message| {
                let fragment = decode_fips_control_frame(message)
                    .expect("decode fragment")
                    .expect("fragment frame");
                let FipsControlFrame::Fragment {
                    id,
                    index,
                    total,
                    data,
                } = fragment
                else {
                    panic!("expected fragment");
                };
                (id, index, total, data)
            })
            .collect();

        let mut buffer = FipsControlFragmentBuffer::default();
        let source_a = [1u8; 16];
        let source_b = [2u8; 16];

        for (offset, (id, index, total, data)) in fragments.iter().cloned().enumerate() {
            let source = if offset == 0 { source_a } else { source_b };
            assert!(
                buffer
                    .push(source, id, index, total, data, 1)
                    .expect("push mixed source fragment")
                    .is_none()
            );
        }

        let mut reassembled = None;
        for (id, index, total, data) in fragments.into_iter().skip(1) {
            reassembled = buffer
                .push(source_a, id, index, total, data, 1)
                .expect("push same source fragment")
                .or(reassembled);
        }
        let decoded = decode_fips_control_frame(&reassembled.expect("reassembled frame"))
            .expect("decode reassembled")
            .expect("control frame");
        assert_eq!(decoded, frame);
    }

    #[test]
    fn unknown_kind_is_dropped_silently() {
        let mut bytes = Vec::from(FIPS_CONTROL_MAGIC);
        bytes.extend_from_slice(br#"{"v":1,"frame":{"kind":"future_kind","x":1}}"#);

        assert!(decode_fips_control_frame(&bytes).expect("decode").is_none());
    }

    #[test]
    fn future_version_is_dropped_silently() {
        let mut bytes = Vec::from(FIPS_CONTROL_MAGIC);
        bytes
            .extend_from_slice(br#"{"v":99,"frame":{"kind":"ping","network_id":"x","sent_at":1}}"#);

        assert!(decode_fips_control_frame(&bytes).expect("decode").is_none());
    }
}
