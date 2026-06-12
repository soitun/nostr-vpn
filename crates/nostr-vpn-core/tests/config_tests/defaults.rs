use super::*;
use nostr_vpn_core::config::{PendingOutboundJoinRequest, normalize_runtime_network_id};
use nostr_vpn_core::fips_control::{NetworkRoster, SignedRoster};

include!("defaults/generated_and_defaults.rs");
include!("defaults/roster_apply.rs");
include!("defaults/aliases_and_join.rs");
