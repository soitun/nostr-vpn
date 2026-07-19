use super::{persistence::*, *};
use crate::paid_routes::PAID_ROUTE_OFFER_VERSION;

const PAID_ROUTE_SESSION_ID_MAX_LEN: usize = 256;

impl PaidRouteStore {
    pub fn build_buyer_session_open(
        &self,
        session_id: &str,
        buyer_npub: &str,
        now_unix: u64,
    ) -> Result<PaidRouteSessionOpen> {
        let buyer_npub = normalize_paid_route_npub(buyer_npub, "buyer")?;
        let session_id = trimmed_required(session_id, "paid route session id")?;
        let session = self
            .sessions
            .get(&session_id)
            .ok_or_else(|| anyhow!("paid route buyer session {session_id} does not exist"))?;
        let lease = self
            .leases
            .get(&session.session.lease_id)
            .ok_or_else(|| anyhow!("paid route buyer session {session_id} has no lease"))?;
        if normalize_paid_route_npub(&lease.lease.buyer_npub, "buyer")? != buyer_npub {
            return Err(anyhow!(
                "paid route session buyer does not match authenticated buyer"
            ));
        }
        let channel = self
            .channels
            .get(&session.session.payment.channel_id)
            .ok_or_else(|| anyhow!("paid route buyer session {session_id} has no channel"))?;
        if channel.role != PaidRouteChannelRole::Buyer {
            return Err(anyhow!("paid route session is not a buyer session"));
        }
        let offer = self.buyer_offer_for_session(lease, channel)?;
        if offer.channel.free_probe_units == 0 {
            return Err(anyhow!("paid route offer does not include a free probe"));
        }
        let expires_at_unix = lease.lease.expires_at_unix.min(channel.expires_at_unix);
        if expires_at_unix <= now_unix {
            return Err(anyhow!("paid route session has expired"));
        }
        Ok(PaidRouteSessionOpen {
            version: PAID_ROUTE_OFFER_VERSION.to_string(),
            service_id: offer.offer_id,
            lease_id: lease.lease.lease_id.clone(),
            channel_id: channel.channel_id.clone(),
            seller_npub: offer.seller_npub,
            expires_at_unix,
        })
    }

    pub fn buyer_session_open_for_seller(
        &self,
        seller_pubkey: &str,
        buyer_npub: &str,
        now_unix: u64,
    ) -> Result<Option<PaidRouteSessionOpen>> {
        let seller_pubkey = normalize_nostr_pubkey(seller_pubkey)?;
        let candidate = self
            .sessions
            .values()
            .filter_map(|session| {
                let lease = self.leases.get(&session.session.lease_id)?;
                let channel = self.channels.get(&session.session.payment.channel_id)?;
                if channel.role != PaidRouteChannelRole::Buyer
                    || normalize_nostr_pubkey(&channel.counterparty_npub)
                        .ok()
                        .as_deref()
                        != Some(seller_pubkey.as_str())
                {
                    return None;
                }
                let expires_at = lease.lease.expires_at_unix.min(channel.expires_at_unix);
                (expires_at > now_unix).then_some((session.updated_at_unix, session))
            })
            .max_by_key(|(updated_at, _)| *updated_at)
            .map(|(_, session)| session);
        candidate
            .map(|session| {
                self.build_buyer_session_open(&session.session.session_id, buyer_npub, now_unix)
            })
            .transpose()
    }

    pub fn apply_seller_session_open(
        &mut self,
        request: ApplyPaidRouteSellerSessionOpenRequest,
    ) -> Result<ApplyPaidRouteSellerSessionOpenResult> {
        let before = self.clone();
        let mut next = before.clone();
        let mut result = next.apply_seller_session_open_inner(request)?;
        result.changed = next != before;
        *self = next;
        Ok(result)
    }

    fn apply_seller_session_open_inner(
        &mut self,
        request: ApplyPaidRouteSellerSessionOpenRequest,
    ) -> Result<ApplyPaidRouteSellerSessionOpenResult> {
        let mut config = request.config;
        config.normalize();
        if !config.enabled || config.channel.free_probe_units == 0 {
            return Err(anyhow!("paid exit does not offer free probes"));
        }
        let open = request.open;
        if open.version != PAID_ROUTE_OFFER_VERSION {
            return Err(anyhow!(
                "unsupported paid route session version {}",
                open.version
            ));
        }
        let seller_npub = normalize_paid_route_npub(&request.seller_npub, "seller")?;
        if normalize_paid_route_npub(&open.seller_npub, "seller")? != seller_npub {
            return Err(anyhow!("paid route session targets a different seller"));
        }
        let buyer_pubkey = normalize_nostr_pubkey(&request.authenticated_buyer_pubkey)?;
        let buyer_npub = PublicKey::parse(&buyer_pubkey)
            .map_err(|error| anyhow!("invalid authenticated paid route buyer: {error}"))?
            .to_bech32()
            .context("failed to encode authenticated buyer npub")?;
        let service_id = validated_session_component(&open.service_id, "service id")?;
        let lease_id = validated_session_component(&open.lease_id, "lease id")?;
        let channel_id = validated_session_component(&open.channel_id, "channel id")?;
        if open.expires_at_unix <= request.now_unix {
            return Err(anyhow!("paid route free probe request has expired"));
        }
        let expires_at_unix = open.expires_at_unix.min(
            request
                .now_unix
                .saturating_add(config.channel.channel_expiry_secs.max(1)),
        );

        for existing in self.sessions.values() {
            let Some(existing_lease) = self.leases.get(&existing.session.lease_id) else {
                continue;
            };
            let Some(existing_channel) = self.channels.get(&existing.session.payment.channel_id)
            else {
                continue;
            };
            if existing_channel.role == PaidRouteChannelRole::Seller
                && normalize_nostr_pubkey(&existing_lease.lease.buyer_npub)
                    .ok()
                    .as_deref()
                    == Some(buyer_pubkey.as_str())
                && existing.session.lease_id != lease_id
                && existing.session.payment.cashu_spilman_payment.is_none()
                && existing.session.payment.cashu_token_lease.is_none()
            {
                return Err(anyhow!(
                    "paid route buyer already consumed a free probe on this seller"
                ));
            }
        }

        let session_id = seller_session_id_for_lease(&lease_id);
        if self.sessions.contains_key(&session_id) {
            let lease = self
                .leases
                .get(&lease_id)
                .ok_or_else(|| anyhow!("existing paid route probe has no lease"))?;
            if lease.lease.offer_id != service_id
                || normalize_paid_route_npub(&lease.lease.buyer_npub, "buyer")? != buyer_npub
            {
                return Err(anyhow!(
                    "existing paid route session does not match authenticated probe request"
                ));
            }
            let admission = self
                .seller_admission_for_buyer(&config, request.now_unix, &buyer_pubkey)
                .ok_or_else(|| anyhow!("existing paid route probe has no seller admission"))?;
            return Ok(ApplyPaidRouteSellerSessionOpenResult {
                service_id,
                lease_id,
                channel_id,
                session_id,
                buyer_npub,
                seller_npub,
                allow_routing: admission.allow_routing,
                state: admission.state,
                changed: false,
            });
        }
        self.ensure_seller_lease_slot_available(&service_id, &lease_id, &channel_id, &buyer_npub)?;
        let quote_id = seller_quote_id_for_lease(&lease_id);
        let payment = PaidRoutePaymentState {
            mode: PaidRoutePaymentMode::CashuSpilman,
            channel_id: channel_id.clone(),
            cashu_unit: "sat".to_string(),
            capacity_sat: config.channel.max_channel_capacity_sat,
            paid_msat: 0,
            updated_at_unix: request.now_unix,
            cashu_spilman_payment: None,
            cashu_token_lease: None,
        };
        self.upsert_quote(
            PaidRouteQuote {
                quote_id: quote_id.clone(),
                offer_id: service_id.clone(),
                payment_mode: PaidRoutePaymentMode::CashuSpilman,
                channel_capacity_sat: config.channel.max_channel_capacity_sat,
                expires_at_unix,
                receiver_pubkey_hex: normalize_nostr_pubkey(&seller_npub)?,
            },
            request.now_unix,
        );
        self.upsert_lease(
            PaidRouteLease {
                lease_id: lease_id.clone(),
                offer_id: service_id.clone(),
                quote_id,
                buyer_npub: buyer_npub.clone(),
                starts_at_unix: request.now_unix,
                expires_at_unix,
            },
            PaidRouteLifecycleStatus::Probing,
            request.now_unix,
        );
        self.upsert_channel(PaidRouteChannelRecord {
            channel_id: channel_id.clone(),
            offer_id: service_id.clone(),
            role: PaidRouteChannelRole::Seller,
            status: PaidRouteLifecycleStatus::Probing,
            payment: payment.clone(),
            mint_url: String::new(),
            counterparty_npub: buyer_npub.clone(),
            created_at_unix: request.now_unix,
            expires_at_unix,
            updated_at_unix: request.now_unix,
            error: String::new(),
        });
        self.upsert_session(
            PaidRouteSession {
                session_id: session_id.clone(),
                lease_id: lease_id.clone(),
                usage: PaidRouteUsage::default(),
                payment,
                realized_exit_ip: None,
                observed_country_code: None,
                observed_asn: None,
                quality: None,
            },
            request.now_unix,
        );

        let admission = self
            .seller_admission_for_buyer(&config, request.now_unix, &buyer_pubkey)
            .ok_or_else(|| anyhow!("paid route free probe did not create a seller admission"))?;
        Ok(ApplyPaidRouteSellerSessionOpenResult {
            service_id,
            lease_id,
            channel_id,
            session_id,
            buyer_npub,
            seller_npub,
            allow_routing: admission.allow_routing,
            state: admission.state,
            changed: false,
        })
    }

    pub(super) fn replace_seller_probe_channel_for_payment(
        &mut self,
        service_id: &str,
        lease_id: &str,
        payment_channel_id: &str,
        buyer_npub: &str,
    ) -> Result<()> {
        let session_id = seller_session_id_for_lease(lease_id);
        let Some(session) = self.sessions.get(&session_id) else {
            return Ok(());
        };
        let probe_channel_id = session.session.payment.channel_id.clone();
        if probe_channel_id == payment_channel_id {
            return Ok(());
        }
        let Some(channel) = self.channels.get(&probe_channel_id) else {
            return Ok(());
        };
        if channel.role != PaidRouteChannelRole::Seller
            || channel.offer_id != service_id
            || normalize_paid_route_npub(&channel.counterparty_npub, "buyer")? != buyer_npub
            || channel.payment.paid_msat != 0
            || channel.payment.cashu_spilman_payment.is_some()
            || channel.payment.cashu_token_lease.is_some()
        {
            return Ok(());
        }
        self.channels.remove(&probe_channel_id);
        if let Some(session) = self.sessions.get_mut(&session_id) {
            session.session.payment.channel_id = payment_channel_id.to_string();
        }
        Ok(())
    }
}

fn validated_session_component(value: &str, label: &str) -> Result<String> {
    let value = trimmed_required(value, label)?;
    if value.len() > PAID_ROUTE_SESSION_ID_MAX_LEN {
        return Err(anyhow!("paid route {label} is too long"));
    }
    Ok(value)
}
