//! Re-export of the canonical LAN-pairing worker from `nostr-vpn-core`. The
//! The app core owns the UI-facing lifecycle while this module keeps the
//! canonical signed join-request transport in one place.
pub use nostr_vpn_core::lan_pairing::*;
