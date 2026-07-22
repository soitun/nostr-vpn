use std::path::Path;

use anyhow::Result;

use crate::config::{
    AppConfig, maybe_autoconfigure_node, normalize_nostr_pubkey, normalize_runtime_network_id,
};
use crate::fips_control::{JoinRosterControl, SignedRoster};
use crate::signed_rosters::{load_signed_rosters, signed_rosters_file_path, upsert_signed_roster};

/// Applies either a QR or manual join roster using the one validation path
/// shared by desktop daemons and mobile packet tunnels.
pub fn apply_join_roster(
    app: &mut AppConfig,
    control: &JoinRosterControl,
    now: u64,
) -> Result<Option<String>> {
    match app.apply_nostr_join_roster(control, now) {
        Ok(Some(applied)) => Ok(Some(applied.network_id)),
        Ok(None) => app.apply_manual_join_roster(control, now),
        Err(join_error) => match app.apply_manual_join_roster(control, now) {
            Ok(Some(network_id)) => Ok(Some(network_id)),
            Ok(None) => Err(join_error),
            Err(manual_error) => Err(manual_error),
        },
    }
}

/// Applies a join roster and persists both the app configuration and the
/// signed-roster artifact before the caller is allowed to acknowledge it.
pub fn apply_join_roster_durably(
    app: &mut AppConfig,
    config_path: &Path,
    control: &JoinRosterControl,
    now: u64,
) -> Result<Option<String>> {
    if join_roster_is_durably_persisted(config_path, control)? {
        return Ok(None);
    }
    let Some(applied_network_id) = apply_join_roster(app, control, now)? else {
        return Ok(None);
    };
    upsert_signed_roster(
        &signed_rosters_file_path(config_path),
        control.signed_roster.clone(),
    )?;
    maybe_autoconfigure_node(app);
    app.save(config_path)?;
    Ok(Some(applied_network_id))
}

/// Returns true only after the exact roster is present in both durable stores
/// and the original one-time join request has been consumed.
pub fn join_roster_is_durably_persisted(
    config_path: &Path,
    control: &JoinRosterControl,
) -> Result<bool> {
    let network_id = control.signed_roster.network_id()?;
    let roster_event_id = control.signed_roster.artifact_hash();
    let persisted = AppConfig::load(config_path)?;
    let original_request_is_pending = persisted
        .pending_nostr_join_request
        .as_ref()
        .is_some_and(|pending| pending.request.request_secret == control.request_secret);
    if original_request_is_pending
        || !signed_roster_is_current_for_app(&persisted, &network_id, &control.signed_roster)
    {
        return Ok(false);
    }
    let store = load_signed_rosters(&signed_rosters_file_path(config_path))?;
    Ok(store
        .latest_for(&network_id)
        .is_some_and(|signed| signed.artifact_hash() == roster_event_id))
}

fn signed_roster_is_current_for_app(
    app: &AppConfig,
    network_id: &str,
    signed_roster: &SignedRoster,
) -> bool {
    let Ok(signed_by) = signed_roster.signer_pubkey_hex() else {
        return false;
    };
    app.networks.iter().any(|network| {
        normalize_runtime_network_id(&network.network_id)
            == normalize_runtime_network_id(network_id)
            && network.shared_roster_updated_at == signed_roster.signed_at()
            && normalize_nostr_pubkey(&network.shared_roster_signed_by)
                .is_ok_and(|value| value == signed_by)
    })
}
