#[allow(clippy::needless_pass_by_value)]
fn decode_paid_route_command_json_output(
    output: Output,
    command_name: &str,
) -> Result<serde_json::Value> {
    ensure_success(command_name, &output)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let document = extract_json_document(&stdout)?;
    serde_json::from_str(document).context("failed to decode paid route command JSON")
}

fn paid_route_wallet_can_fund_channel(
    wallet: &PaidRouteWalletState,
    mint_url: &str,
    capacity_sat: u64,
) -> bool {
    let required_msat = capacity_sat.saturating_mul(1_000);
    wallet.mints.iter().any(|mint| {
        mint.url.trim() == mint_url.trim()
            && mint
                .balance_msat
                .is_some_and(|balance_msat| balance_msat >= required_msat)
    })
}

fn json_string(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn json_u64(value: &serde_json::Value, key: &str) -> u64 {
    value
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default()
}

fn json_bool(value: &serde_json::Value, key: &str) -> bool {
    value
        .get(key)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or_default()
}

fn paid_route_probe_action_state(value: &serde_json::Value) -> NativePaidRoutePaymentActionState {
    let probe = value.get("probe").unwrap_or(&serde_json::Value::Null);
    let measurement = value.get("measurement").unwrap_or(&serde_json::Value::Null);
    let quality = measurement
        .get("quality")
        .unwrap_or(&serde_json::Value::Null);
    let realized_ip = json_string(measurement, "realized_exit_ip");
    let latency_ms = json_u64(quality, "latency_ms");
    let status_text = if realized_ip.is_empty() {
        "Probe saved".to_string()
    } else if latency_ms > 0 {
        format!("Probe {realized_ip} {latency_ms} ms")
    } else {
        format!("Probe {realized_ip}")
    };

    NativePaidRoutePaymentActionState {
        kind: "probe".to_string(),
        status_text,
        payload_type: "probe".to_string(),
        session_id: json_string(probe, "session_id"),
        ..NativePaidRoutePaymentActionState::default()
    }
}

fn paid_route_payment_action_state(
    kind: &str,
    value: &serde_json::Value,
) -> Result<NativePaidRoutePaymentActionState> {
    let payment = value
        .get("payment")
        .ok_or_else(|| anyhow!("paid route payment output is missing payment"))?;
    let payload_type = json_string(payment, "payload_type");
    let access_state = json_string(payment, "state");
    let envelope_json = payment
        .get("envelope")
        .map(serde_json::to_string)
        .transpose()
        .context("failed to encode paid route payment envelope JSON")?
        .unwrap_or_default();
    let status_text = if access_state.is_empty() {
        payload_type.clone()
    } else if payload_type.is_empty() {
        access_state.clone()
    } else {
        format!("{payload_type} {access_state}")
    };
    let paid_msat = json_u64(payment, "paid_msat");
    let delivered_units = json_u64(payment, "delivered_units");
    let amount_due_msat = json_u64(payment, "amount_due_msat");
    let unpaid_msat = json_u64(payment, "unpaid_msat");

    Ok(NativePaidRoutePaymentActionState {
        kind: kind.to_string(),
        status_text,
        payload_type,
        session_id: json_string(payment, "session_id"),
        lease_id: json_string(payment, "lease_id"),
        channel_id: json_string(payment, "channel_id"),
        buyer_npub: json_string(payment, "buyer_npub"),
        seller_npub: json_string(payment, "seller_npub"),
        envelope_json,
        paid_msat,
        paid_text: paid_route_paid_text(paid_msat),
        delivered_units,
        delivered_usage_text: paid_route_usage_text(delivered_units),
        amount_due_msat,
        amount_due_text: paid_route_due_text(amount_due_msat),
        unpaid_msat,
        unpaid_text: paid_route_unpaid_text(unpaid_msat),
        allow_routing: json_bool(payment, "allow_routing"),
    })
}

fn paid_route_payment_send_action_state(
    value: &serde_json::Value,
) -> NativePaidRoutePaymentActionState {
    let (success_count, failed_count) = paid_route_payment_publish_counts(value);
    let status_text = if success_count > 0 {
        if failed_count > 0 {
            format!("Payment sent to {success_count} relays, {failed_count} failed")
        } else {
            format!("Payment sent to {success_count} relays")
        }
    } else if failed_count > 0 {
        format!("Payment send failed on {failed_count} relays")
    } else {
        "Payment send attempted".to_string()
    };

    NativePaidRoutePaymentActionState {
        kind: "send".to_string(),
        status_text,
        lease_id: json_string(value, "lease_id"),
        channel_id: json_string(value, "channel_id"),
        buyer_npub: json_string(value, "buyer"),
        seller_npub: json_string(value, "seller"),
        ..NativePaidRoutePaymentActionState::default()
    }
}

fn paid_route_payment_publish_counts(value: &serde_json::Value) -> (u64, u64) {
    let publish = value.get("publish").unwrap_or(value);
    let publish = publish.get("result").unwrap_or(publish);
    (
        json_u64(publish, "success_count"),
        json_u64(publish, "failed_count"),
    )
}

fn paid_route_payment_stream_action_state(
    value: &serde_json::Value,
) -> Result<NativePaidRoutePaymentActionState> {
    let signed_count = json_u64(value, "signed_count");
    let persisted_count = json_u64(value, "persisted_count");
    let error_count = json_u64(value, "error_count");
    let total_due_count = json_u64(value, "total_due_count");
    let publish_requested = json_bool(value, "publish_requested");
    let status_text = paid_route_stream_status_text(
        signed_count,
        persisted_count,
        error_count,
        total_due_count,
        publish_requested,
    );

    if let Some(payment) = value
        .get("signed")
        .and_then(serde_json::Value::as_array)
        .and_then(|signed| signed.first())
        .and_then(|entry| entry.get("payment"))
    {
        let mut state = paid_route_payment_action_state("stream", &json!({ "payment": payment }))?;
        state.status_text = status_text;
        return Ok(state);
    }

    Ok(NativePaidRoutePaymentActionState {
        kind: "stream".to_string(),
        status_text,
        ..NativePaidRoutePaymentActionState::default()
    })
}

fn paid_route_stream_status_text(
    signed_count: u64,
    persisted_count: u64,
    error_count: u64,
    total_due_count: u64,
    publish_requested: bool,
) -> String {
    let verb = if publish_requested {
        "Streamed"
    } else {
        "Signed"
    };
    if signed_count == 0 && error_count == 0 {
        return if total_due_count == 0 {
            "No payment updates due".to_string()
        } else {
            "No payment updates streamed".to_string()
        };
    }
    if error_count == 0 {
        return match signed_count {
            1 => format!("{verb} 1 payment update"),
            count => format!("{verb} {count} payment updates"),
        };
    }
    if signed_count == 0 {
        return match error_count {
            1 => "1 payment update failed".to_string(),
            count => format!("{count} payment updates failed"),
        };
    }

    let completed = if publish_requested {
        persisted_count
    } else {
        signed_count
    };
    match (completed, error_count) {
        (1, 1) => format!("{verb} 1 payment update, 1 failed"),
        (1, errors) => format!("{verb} 1 payment update, {errors} failed"),
        (count, 1) => format!("{verb} {count} payment updates, 1 failed"),
        (count, errors) => format!("{verb} {count} payment updates, {errors} failed"),
    }
}

fn paid_route_payment_receive_action_state(
    value: &serde_json::Value,
) -> Result<NativePaidRoutePaymentActionState> {
    let applied_count = json_u64(value, "applied_count");
    let error_count = json_u64(value, "error_count");
    if let Some(payment) = value
        .get("applied")
        .and_then(serde_json::Value::as_array)
        .and_then(|applied| applied.first())
        .and_then(|entry| entry.get("payment"))
    {
        let mut state =
            paid_route_payment_action_state("receive", &json!({ "payment": payment }))?;
        state.status_text = paid_route_receive_status_text(applied_count, error_count);
        return Ok(state);
    }

    Ok(NativePaidRoutePaymentActionState {
        kind: "receive".to_string(),
        status_text: paid_route_receive_status_text(applied_count, error_count),
        seller_npub: json_string(value, "seller"),
        ..NativePaidRoutePaymentActionState::default()
    })
}

fn paid_route_payment_collect_action_state(
    value: &serde_json::Value,
) -> NativePaidRoutePaymentActionState {
    let close = value
        .get("spilman_close")
        .unwrap_or(&serde_json::Value::Null);
    let receiver_amount_sat = json_u64(close, "receiver_amount_sat");
    let paid_msat = receiver_amount_sat.saturating_mul(1_000);
    let status_text = if json_bool(close, "already_closed") {
        if receiver_amount_sat > 0 {
            format!(
                "Already collected {}",
                paid_route_sat_text(receiver_amount_sat)
            )
        } else {
            "Already collected".to_string()
        }
    } else if receiver_amount_sat > 0 {
        format!("Collected {}", paid_route_sat_text(receiver_amount_sat))
    } else {
        "Channel collected".to_string()
    };

    NativePaidRoutePaymentActionState {
        kind: "collect".to_string(),
        status_text,
        payload_type: "spilman_close".to_string(),
        channel_id: json_string(close, "channel_id"),
        paid_msat,
        paid_text: paid_route_paid_text(paid_msat),
        allow_routing: false,
        ..NativePaidRoutePaymentActionState::default()
    }
}

fn paid_route_payment_collect_due_action_state(
    value: &serde_json::Value,
) -> NativePaidRoutePaymentActionState {
    let collected_count = json_u64(value, "collected_count");
    let error_count = json_u64(value, "error_count");
    let paid_msat = value
        .get("collected")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("spilman_close"))
        .map(|close| json_u64(close, "receiver_amount_sat").saturating_mul(1_000))
        .fold(0_u64, u64::saturating_add);
    let status_text = paid_route_collect_due_status_text(collected_count, error_count, paid_msat);

    NativePaidRoutePaymentActionState {
        kind: "collect".to_string(),
        status_text,
        payload_type: "spilman_close".to_string(),
        paid_msat,
        paid_text: paid_route_paid_text(paid_msat),
        allow_routing: false,
        ..NativePaidRoutePaymentActionState::default()
    }
}

fn paid_route_collect_due_status_text(
    collected_count: u64,
    error_count: u64,
    paid_msat: u64,
) -> String {
    if collected_count == 0 && error_count == 0 {
        return "No channels due to collect".to_string();
    }
    let collected_text = match collected_count {
        0 => String::new(),
        1 => format!("Collected 1 channel ({})", paid_route_msat_text(paid_msat)),
        count => format!(
            "Collected {count} channels ({})",
            paid_route_msat_text(paid_msat)
        ),
    };
    if error_count == 0 {
        return collected_text;
    }
    if collected_count == 0 {
        return match error_count {
            1 => "1 channel failed to collect".to_string(),
            count => format!("{count} channels failed to collect"),
        };
    }
    match error_count {
        1 => format!("{collected_text}, 1 failed"),
        count => format!("{collected_text}, {count} failed"),
    }
}

fn paid_route_receive_status_text(applied_count: u64, error_count: u64) -> String {
    match (applied_count, error_count) {
        (0, 0) => "No payment updates received".to_string(),
        (1, 0) => "Received 1 payment update".to_string(),
        (count, 0) => format!("Received {count} payment updates"),
        (0, 1) => "1 payment update failed".to_string(),
        (0, count) => format!("{count} payment updates failed"),
        (applied, errors) => {
            format!("Received {applied} payment updates, {errors} failed")
        }
    }
}
