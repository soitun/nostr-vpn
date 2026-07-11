use anyhow::{Context, Result, anyhow};
use nostr_sdk::prelude::Keys;
use serde::{Deserialize, Serialize};

use crate::config::{AppConfig, normalize_nostr_pubkey, normalize_runtime_network_id};
use crate::identity_bridge::{
    NostrIdentityDeviceApprovalRequest, nostr_identity_device_approval_bootstrap,
};

pub const FIPS_JOIN_REQUEST_RETRY_SECS: u64 = 10;
pub const NOSTR_VPN_JOIN_REQUEST_TYPE: &str = "nostr-vpn.join-request";

/// Legacy persisted request state. nVPN no longer creates or transports this;
/// keeping the shape allows old configs to load so the caller can clear it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingNostrJoinRequest {
    pub request: NostrIdentityDeviceApprovalRequest,
    pub request_private_key: String,
}

impl PendingNostrJoinRequest {
    pub fn validate_for_device(&self, device_app_key_pubkey: &str) -> Result<()> {
        let expected_device = normalize_nostr_pubkey(device_app_key_pubkey)?;
        nostr_identity_device_approval_bootstrap(&self.request)
            .map_err(|error| anyhow!("pending join request is invalid: {error}"))?;
        if self.request.device_app_key_pubkey != expected_device {
            return Err(anyhow!(
                "pending join request device AppKey does not match local identity"
            ));
        }
        if self.request.request_type.as_deref() != Some(NOSTR_VPN_JOIN_REQUEST_TYPE) {
            return Err(anyhow!("pending join request has invalid request type"));
        }
        let keys = Keys::parse(self.request_private_key.trim())
            .context("pending join request private key is invalid")?;
        if keys.public_key().to_hex() != self.request.request_pubkey {
            return Err(anyhow!(
                "pending join request private key does not match request pubkey"
            ));
        }
        Ok(())
    }
}

impl AppConfig {
    pub fn clear_pending_nostr_join_request(&mut self) -> bool {
        self.pending_nostr_join_request.take().is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeshJoinRequest {
    pub network_id: String,
    #[serde(default)]
    pub invite_secret: String,
    #[serde(default)]
    pub requester_node_name: String,
}

pub fn normalize_join_request(request: MeshJoinRequest) -> Result<MeshJoinRequest> {
    let network_id = normalize_runtime_network_id(&request.network_id);
    if network_id.is_empty() {
        return Err(anyhow!("mesh join request network_id must not be empty"));
    }

    Ok(MeshJoinRequest {
        network_id,
        invite_secret: request.invite_secret.trim().to_string(),
        requester_node_name: request.requester_node_name.trim().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_request_normalizes_network_id_and_node_name() {
        let request = normalize_join_request(MeshJoinRequest {
            network_id: "  Mesh Home  ".to_string(),
            invite_secret: " invite-secret ".to_string(),
            requester_node_name: " alice-phone ".to_string(),
        })
        .expect("normalize");

        assert_eq!(request.network_id, "Mesh Home");
        assert_eq!(request.invite_secret, "invite-secret");
        assert_eq!(request.requester_node_name, "alice-phone");
    }
}
