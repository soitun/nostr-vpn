use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use futures::future::join_all;
use nostr_pubsub::{EventBus, EventSource, QueryOptions, VerifiedEvent};
use nostr_pubsub_relay::RelayEventBus;
use nostr_sdk::prelude::{
    Alphabet, Client, Event, Filter, Kind, PublicKey, RelayPoolNotification, SingleLetterTag,
    Timestamp,
};
use nostr_vpn_core::config::AppConfig;
#[cfg(not(test))]
use nostr_vpn_core::join_requests::NOSTR_VPN_JOIN_APPROVAL_RELAY;

#[cfg(not(test))]
pub async fn publish_join_approval_events(config: &AppConfig, events: &[Event]) -> Result<()> {
    publish_join_approval_events_to_relay(config, events, NOSTR_VPN_JOIN_APPROVAL_RELAY).await
}

async fn publish_join_approval_events_to_relay(
    config: &AppConfig,
    events: &[Event],
    relay_url: &str,
) -> Result<()> {
    let client = Client::new(config.nostr_keys()?);
    let provider = RelayEventBus::with_client(client, [relay_url], Duration::from_secs(10))
        .await
        .map_err(|error| anyhow!("failed to initialize join approval pubsub provider: {error}"))?;
    let events = events
        .iter()
        .cloned()
        .map(VerifiedEvent::try_from)
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("join request approval event failed signature verification")?;
    let publishes = events
        .into_iter()
        .map(|event| provider.publish(event, EventSource::local_index("nostr-vpn-join-approval")));
    let reports = tokio::time::timeout(Duration::from_secs(30), join_all(publishes)).await;
    provider.client().disconnect().await;
    let reports = reports.context("join request approval pubsub batch timed out")?;
    for report in reports {
        let report = report
            .map_err(|error| anyhow!("failed to publish join request approval event: {error}"))?;
        if !report.accepted {
            return Err(anyhow!(
                "join request approval pubsub provider rejected a verified event"
            ));
        }
    }
    Ok(())
}

#[cfg(not(test))]
pub async fn fetch_pending_join_approval_events(
    config: &AppConfig,
    cancelled: tokio::sync::oneshot::Receiver<()>,
) -> Result<Vec<Event>> {
    fetch_pending_join_approval_events_from_relay_until_cancelled(
        config,
        NOSTR_VPN_JOIN_APPROVAL_RELAY,
        Duration::from_secs(20),
        async {
            let _ = cancelled.await;
        },
    )
    .await
}

#[cfg(test)]
async fn fetch_pending_join_approval_events_from_relay(
    config: &AppConfig,
    relay_url: &str,
    wait: Duration,
) -> Result<Vec<Event>> {
    fetch_pending_join_approval_events_from_relay_until_cancelled(
        config,
        relay_url,
        wait,
        std::future::pending(),
    )
    .await
}

async fn fetch_pending_join_approval_events_from_relay_until_cancelled(
    config: &AppConfig,
    relay_url: &str,
    wait: Duration,
    cancelled: impl std::future::Future<Output = ()>,
) -> Result<Vec<Event>> {
    let pending = config
        .pending_nostr_join_request
        .as_ref()
        .ok_or_else(|| anyhow!("no pending Nostr join request"))?;
    pending.validate_for_device(&config.own_nostr_pubkey_hex()?)?;
    let request_pubkey = PublicKey::parse(&pending.request.request_pubkey)
        .context("pending Nostr join request pubkey is invalid")?;
    let requested_at = u64::try_from(pending.request.requested_at)
        .context("pending Nostr join request timestamp is negative")?;
    let filter = Filter::new()
        .kind(Kind::Custom(7_368))
        .custom_tag(
            SingleLetterTag::lowercase(Alphabet::P),
            request_pubkey.to_hex(),
        )
        .since(Timestamp::from(requested_at))
        .limit(8);

    let client = Client::new(config.nostr_keys()?);
    tokio::pin!(cancelled);
    let provider = tokio::select! {
        () = &mut cancelled => return Err(join_approval_cancelled()),
        result = RelayEventBus::with_client(client, [relay_url], Duration::from_secs(10)) => {
            result.map_err(|error| anyhow!("failed to initialize join approval subscriber: {error}"))?
        }
    };
    let client = provider.client();
    let mut notifications = client.notifications();
    tokio::select! {
        () = &mut cancelled => {
            client.disconnect().await;
            return Err(join_approval_cancelled());
        }
        result = client.subscribe(filter.clone(), None) => {
            result.context("failed to subscribe for join approval")?;
        }
    }

    let report = tokio::select! {
        () = &mut cancelled => {
            client.disconnect().await;
            return Err(join_approval_cancelled());
        }
        result = provider.query(vec![filter], QueryOptions { limit: Some(8) }) => {
            result.map_err(|error| anyhow!("failed to fetch join approval events: {error}"))?
        }
    };
    let mut events = report
        .events
        .into_iter()
        .map(|candidate| candidate.event.into_event())
        .filter(|event| approval_event_targets(event, &request_pubkey.to_hex()))
        .collect::<Vec<_>>();
    if approval_events_complete(config, &events) {
        client.disconnect().await;
        return Ok(events);
    }

    let timeout = tokio::time::sleep(wait);
    tokio::pin!(timeout);
    loop {
        tokio::select! {
            () = &mut cancelled => {
                client.disconnect().await;
                return Err(join_approval_cancelled());
            }
            () = &mut timeout => break,
            notification = notifications.recv() => {
                match notification {
                    Ok(RelayPoolNotification::Event { event, .. }) => {
                        if approval_event_targets(&event, &request_pubkey.to_hex())
                            && !events.iter().any(|known| known.id == event.id)
                        {
                            events.push((*event).clone());
                            if approval_events_complete(config, &events) {
                                break;
                            }
                        }
                    }
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
    client.disconnect().await;
    Ok(events)
}

fn join_approval_cancelled() -> anyhow::Error {
    anyhow!("join approval subscription cancelled")
}

fn approval_event_targets(event: &Event, request_pubkey: &str) -> bool {
    event.kind.as_u16() == 7_368
        && event.tags.iter().any(|tag| {
            let parts = tag.as_slice();
            parts.first().is_some_and(|part| part == "p")
                && parts.get(1).is_some_and(|part| part == request_pubkey)
        })
}

fn approval_events_complete(config: &AppConfig, events: &[Event]) -> bool {
    let mut candidate = config.clone();
    candidate
        .apply_nostr_join_approval_events(events, unix_timestamp())
        .is_ok_and(|applied| applied.is_some())
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use futures::{SinkExt, StreamExt};
    use nostr_sdk::prelude::Keys;
    use nostr_vpn_core::identity_bridge::nostr_identity_device_approval_bootstrap;
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;
    use tokio_tungstenite::tungstenite::Message;

    use super::*;
    use crate::join_approval::prepare_join_approval;

    #[tokio::test]
    async fn cancelled_join_request_stops_listening_on_the_relay() {
        let mut joiner = AppConfig::generated_without_networks();
        joiner
            .ensure_pending_nostr_join_request(unix_timestamp())
            .expect("pending join request");
        let relay = LocalNostrRelay::spawn().await;
        let relay_url = relay.url.clone();
        let (cancel, cancelled) = oneshot::channel();
        let fetch = tokio::spawn(async move {
            fetch_pending_join_approval_events_from_relay_until_cancelled(
                &joiner,
                &relay_url,
                Duration::from_secs(30),
                async {
                    let _ = cancelled.await;
                },
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!fetch.is_finished(), "listener stopped before cancellation");

        cancel.send(()).expect("cancel join request listener");
        let error = tokio::time::timeout(Duration::from_secs(1), fetch)
            .await
            .expect("listener did not stop promptly")
            .expect("listener task panicked")
            .expect_err("cancelled listener should not return approval events");

        assert!(error.to_string().contains("cancelled"), "{error:#}");
        relay.stop().await;
    }

    #[tokio::test]
    async fn native_join_approval_round_trips_through_relay() {
        let requested_at = unix_timestamp();
        let mut joiner = AppConfig::generated_without_networks();
        joiner.node_name = "virus.exe".to_string();
        joiner
            .ensure_pending_nostr_join_request(requested_at)
            .expect("pending iPhone join request");
        let bootstrap = nostr_identity_device_approval_bootstrap(
            &joiner
                .pending_nostr_join_request
                .as_ref()
                .expect("pending request")
                .request,
        )
        .expect("join request bootstrap");
        let joiner_pubkey = joiner.own_nostr_pubkey_hex().expect("joiner pubkey");

        let admin_keys = Keys::generate();
        let mut admin = AppConfig::generated();
        admin.nostr.secret_key = admin_keys.secret_key().to_secret_hex();
        admin.nostr.public_key = admin_keys.public_key().to_hex();
        admin.networks[0].enabled = true;
        admin.networks[0].name = "Home".to_string();
        admin.networks[0].network_id = "8d4f34f5425bc50e".to_string();
        admin.networks[0].devices = vec![admin_keys.public_key().to_hex()];
        admin.networks[0].admins = vec![admin_keys.public_key().to_hex()];
        admin.ensure_defaults();
        let network_id = admin.networks[0].id.clone();
        let prepared = prepare_join_approval(
            &admin,
            &network_id,
            &bootstrap,
            requested_at.saturating_add(1),
        )
        .expect("prepare approval");

        let relay = LocalNostrRelay::spawn().await;
        publish_join_approval_events_to_relay(&admin, &prepared.events, &relay.url)
            .await
            .expect("publish approval events");
        let fetched = fetch_pending_join_approval_events_from_relay(
            &joiner,
            &relay.url,
            Duration::from_secs(2),
        )
        .await
        .expect("fetch approval events");
        let applied = joiner
            .apply_nostr_join_approval_events(&fetched, requested_at.saturating_add(2))
            .expect("apply approval")
            .expect("approval detected");

        assert_eq!(applied.network_id, "8d4f34f5425bc50e");
        assert!(joiner.pending_nostr_join_request.is_none());
        assert!(joiner.active_network_has_confirmed_local_identity());
        assert_eq!(joiner.active_network().name, "Home");
        assert_eq!(joiner.own_nostr_pubkey_hex().unwrap(), joiner_pubkey);
        relay.stop().await;
    }

    struct LocalNostrRelay {
        url: String,
        shutdown: Option<oneshot::Sender<()>>,
        handle: tokio::task::JoinHandle<()>,
    }

    impl LocalNostrRelay {
        async fn spawn() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind local relay");
            let url = format!("ws://{}", listener.local_addr().expect("relay address"));
            let events = Arc::new(Mutex::new(Vec::new()));
            let (shutdown, shutdown_rx) = oneshot::channel();
            let handle = tokio::spawn(run_local_nostr_relay(listener, events, shutdown_rx));
            Self {
                url,
                shutdown: Some(shutdown),
                handle,
            }
        }

        async fn stop(mut self) {
            if let Some(shutdown) = self.shutdown.take() {
                let _ = shutdown.send(());
            }
            let _ = self.handle.await;
        }
    }

    async fn run_local_nostr_relay(
        listener: TcpListener,
        events: Arc<Mutex<Vec<serde_json::Value>>>,
        mut shutdown: oneshot::Receiver<()>,
    ) {
        loop {
            tokio::select! {
                _ = &mut shutdown => break,
                accepted = listener.accept() => {
                    let Ok((stream, _)) = accepted else {
                        continue;
                    };
                    let events = Arc::clone(&events);
                    tokio::spawn(async move {
                        let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await else {
                            return;
                        };
                        while let Some(Ok(message)) = socket.next().await {
                            let Message::Text(text) = message else {
                                continue;
                            };
                            let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                                continue;
                            };
                            let Some(items) = value.as_array() else {
                                continue;
                            };
                            match items.first().and_then(serde_json::Value::as_str) {
                                Some("EVENT") => {
                                    if let Some(event) = items.get(1).cloned() {
                                        let event_id = event
                                            .get("id")
                                            .and_then(serde_json::Value::as_str)
                                            .unwrap_or_default()
                                            .to_string();
                                        events.lock().expect("relay event lock").push(event);
                                        let response = serde_json::json!(["OK", event_id, true, ""]);
                                        let _ = socket
                                            .send(Message::Text(response.to_string().into()))
                                            .await;
                                    }
                                }
                                Some("REQ") => {
                                    let Some(subscription_id) =
                                        items.get(1).and_then(serde_json::Value::as_str)
                                    else {
                                        continue;
                                    };
                                    let snapshot = events.lock().expect("relay event lock").clone();
                                    for event in snapshot {
                                        let response = serde_json::json!([
                                            "EVENT",
                                            subscription_id,
                                            event
                                        ]);
                                        let _ = socket
                                            .send(Message::Text(response.to_string().into()))
                                            .await;
                                    }
                                    let eose = serde_json::json!(["EOSE", subscription_id]);
                                    let _ = socket
                                        .send(Message::Text(eose.to_string().into()))
                                        .await;
                                }
                                Some("CLOSE") => break,
                                _ => {}
                            }
                        }
                    });
                }
            }
        }
    }
}
