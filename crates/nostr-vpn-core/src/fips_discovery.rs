use sha2::{Digest, Sha256};

pub const FIPS_LAN_DISCOVERY_SCOPE_PREFIX: &str = "nostr-vpn";

/// Derive the private LAN discovery scope shared by every nvpn platform.
///
/// The network ID must not appear in public mDNS records, and every client
/// must derive the exact same opaque scope or nearby peers silently ignore
/// one another.
pub fn fips_lan_discovery_scope(network_id: &str) -> String {
    let digest = Sha256::digest(network_id.trim().as_bytes());
    format!(
        "{FIPS_LAN_DISCOVERY_SCOPE_PREFIX}:{}",
        hex::encode(&digest[..16])
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lan_discovery_scope_is_private_stable_and_trimmed() {
        let scope = fips_lan_discovery_scope(" private-network-id ");

        assert!(scope.starts_with(&format!("{FIPS_LAN_DISCOVERY_SCOPE_PREFIX}:")));
        assert!(!scope.contains("private-network-id"));
        assert_eq!(scope, fips_lan_discovery_scope("private-network-id"));
    }
}
