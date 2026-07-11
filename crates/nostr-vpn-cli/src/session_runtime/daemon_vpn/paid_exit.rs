use super::*;

pub(super) const PAID_EXIT_DAEMON_STREAM_PAYMENT_MIN_INCREMENT_MSAT: u64 = 1;
pub(super) const PAID_EXIT_DAEMON_STREAM_PAYMENT_LIMIT: usize = 4;
pub(super) const PAID_EXIT_DAEMON_RECEIVE_PAYMENT_INTERVAL_SECS: u64 = 5;
pub(super) const PAID_EXIT_DAEMON_RECEIVE_PAYMENT_DURATION_SECS: u64 = 2;
pub(super) const PAID_EXIT_DAEMON_RECEIVE_PAYMENT_LIMIT: usize = 100;

pub(super) fn flush_fips_paid_route_usage(
    runtime: &crate::fips_private_mesh::FipsPrivateTunnelRuntime,
    app: &AppConfig,
    config_path: &Path,
    now_unix: u64,
    active_millis_delta: u64,
) -> Result<bool> {
    let store_path = paid_route_store_file_path(config_path);
    let mut store = load_paid_route_store(&store_path)?;
    let mut changed = false;
    let seller_admission_routing_before = if app.paid_exit.enabled {
        paid_route_seller_admission_routing_signature(
            &store.seller_admissions(&app.paid_exit, now_unix),
        )
    } else {
        Vec::new()
    };

    if let Some(seller_pubkey) = app.public_paid_exit_node_pubkey_hex() {
        let mut usage_delta = runtime.drain_paid_route_usage(&seller_pubkey)?;
        usage_delta.active_millis = usage_delta
            .active_millis
            .saturating_add(active_millis_delta);
        if !usage_delta.is_empty() {
            changed |= store
                .record_buyer_usage(RecordPaidRouteBuyerUsageRequest {
                    seller_pubkey,
                    usage_delta,
                    now_unix,
                })?
                .is_some_and(|result| result.changed);
        }
    }

    if app.paid_exit.enabled {
        for admission in store.seller_admissions(&app.paid_exit, now_unix) {
            let mut usage_delta = runtime.drain_paid_route_usage(&admission.buyer_pubkey)?;
            if admission.allow_routing {
                usage_delta.active_millis = usage_delta
                    .active_millis
                    .saturating_add(active_millis_delta);
            }
            if usage_delta.is_empty() {
                continue;
            }
            changed |= store
                .record_seller_usage(RecordPaidRouteSellerUsageRequest {
                    buyer_pubkey: admission.buyer_pubkey,
                    config: app.paid_exit.clone(),
                    usage_delta,
                    now_unix,
                })?
                .is_some_and(|result| result.changed);
        }
    }

    if changed {
        write_paid_route_store(&store_path, &store)?;
    }
    let seller_admission_routing_after = if changed && app.paid_exit.enabled {
        paid_route_seller_admission_routing_signature(
            &store.seller_admissions(&app.paid_exit, now_unix),
        )
    } else {
        seller_admission_routing_before.clone()
    };
    Ok(seller_admission_routing_after != seller_admission_routing_before)
}

fn paid_route_seller_admission_routing_signature(
    admissions: &[nostr_vpn_core::paid_route_store::PaidRouteSellerAdmission],
) -> Vec<(String, String, bool)> {
    admissions
        .iter()
        .map(|admission| {
            (
                admission.buyer_pubkey.clone(),
                admission.session_id.clone(),
                admission.allow_routing,
            )
        })
        .collect()
}
