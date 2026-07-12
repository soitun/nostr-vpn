pub mod actions;
pub mod c_abi;
mod exchange_rate;
mod ffi;
mod invite;
pub mod join_approval;
mod join_approval_transport;
pub mod join_request_link;
pub mod lan_pairing;
mod mobile_tunnel;
pub mod native_state;
pub mod platform;
pub mod state;
mod wg_upstream_nat;

pub use actions::NativeAppAction;
pub use ffi::FfiApp;
pub use native_state::{NativeAppState, NativeNetworkState, NativeParticipantState};
pub use nostr_vpn_core::updater::UpdateAutoCheckPolicy;
pub use platform::{
    NativeRuntimeCapabilities, RuntimePlatform, current_runtime_capabilities,
    current_runtime_platform, runtime_capabilities_for,
};
pub use state::{
    DaemonPeerState, DaemonRuntimeState, InboundJoinRequestView, LanPeerView, NetworkView,
    OutboundJoinRequestView, ParticipantView, SettingsPatch, TrayExitNodeEntry, TrayMenuItemSpec,
    TrayNetworkGroup, TrayRuntimeState, UiState,
};

uniffi::setup_scaffolding!();
