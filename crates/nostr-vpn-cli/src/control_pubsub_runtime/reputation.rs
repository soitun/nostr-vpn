use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use nostr_pubsub_social_graph::{MeshPeerPolicy, SocialGraphPolicy, SocialGraphPolicyConfig};
use nostr_sdk::prelude::{Event, Kind, PublicKey};
use nostr_social_graph::{Rating, RatingGraphConfig, SocialGraph};
use nostr_social_memory::rating_from_event;
use nostr_vpn_core::control_pubsub::RATING_FACT_KIND;

pub(super) const DEFAULT_PEER_RATING_SCOPE: &str = "fips.peer";
pub(super) const RATING_EVALUATION_INTERVAL: Duration = Duration::from_secs(60);
const RATING_MIN_PUBLISH_INTERVAL_MS: u64 = 10 * 60 * 1_000;
const RATING_REFRESH_INTERVAL_MS: u64 = 24 * 60 * 60 * 1_000;
const RATING_MATERIAL_SCORE_DELTA: i64 = 20;
const MIN_POSITIVE_RATING_SAMPLES: u64 = 3;
pub(super) const RATING_PUBLISH_BATCH: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PeerRatingKey {
    rater: String,
    subject: String,
    scope: String,
}

#[derive(Debug, Clone)]
struct StoredPeerRating {
    event_id: String,
    rating: Rating,
}

pub(super) struct DefaultPeerReputation {
    pub(super) root: String,
    graph: Arc<RwLock<SocialGraph>>,
    latest: BTreeMap<PeerRatingKey, StoredPeerRating>,
}

impl DefaultPeerReputation {
    pub(super) fn new(local_npub: &str) -> Result<(Self, Arc<dyn MeshPeerPolicy>)> {
        let root = PublicKey::parse(local_npub)
            .context("invalid local FIPS identity for peer reputation")?
            .to_hex();
        let graph = Arc::new(RwLock::new(SocialGraph::new(&root)));
        let policy: Arc<dyn MeshPeerPolicy> = Arc::new(SocialGraphPolicy::new(
            Arc::clone(&graph),
            SocialGraphPolicyConfig::default(),
        ));
        Ok((
            Self {
                root,
                graph,
                latest: BTreeMap::new(),
            },
            policy,
        ))
    }

    pub(super) fn ingest_event(&mut self, event: &Event) -> Result<bool> {
        if !self.consider_event(event) {
            return Ok(false);
        }
        self.rebuild()?;
        Ok(true)
    }

    pub(super) fn replay<'a>(
        &mut self,
        events: impl IntoIterator<Item = &'a Event>,
    ) -> Result<usize> {
        let mut changed = 0usize;
        for event in events {
            changed += usize::from(self.consider_event(event));
        }
        if changed > 0 {
            self.rebuild()?;
        }
        Ok(changed)
    }

    fn consider_event(&mut self, event: &Event) -> bool {
        if event.kind != Kind::Custom(RATING_FACT_KIND) || event.verify().is_err() {
            return false;
        }
        let Ok(mut rating) = rating_from_event(event) else {
            return false;
        };
        if rating.scope.as_deref() != Some(DEFAULT_PEER_RATING_SCOPE) {
            return false;
        }
        let Ok(rater) = PublicKey::parse(&rating.rater) else {
            return false;
        };
        if rater != event.pubkey {
            return false;
        }
        let Ok(subject) = PublicKey::parse(&rating.subject) else {
            return false;
        };
        rating.rater = rater.to_hex();
        rating.subject = subject.to_hex();
        let key = PeerRatingKey {
            rater: rating.rater.clone(),
            subject: rating.subject.clone(),
            scope: DEFAULT_PEER_RATING_SCOPE.to_string(),
        };
        let event_id = event.id.to_hex();
        if self.latest.get(&key).is_some_and(|existing| {
            (existing.rating.created_at, &existing.event_id) >= (rating.created_at, &event_id)
        }) {
            return false;
        }
        self.latest
            .insert(key, StoredPeerRating { event_id, rating });
        true
    }

    fn rebuild(&self) -> Result<()> {
        let mut graph = SocialGraph::new(&self.root);
        let ratings = self
            .latest
            .values()
            .map(|stored| stored.rating.clone())
            .collect::<Vec<_>>();
        graph
            .apply_ratings(
                &ratings,
                &RatingGraphConfig::for_scopes([DEFAULT_PEER_RATING_SCOPE]),
            )
            .context("failed to apply peer ratings")?;
        *self
            .graph
            .write()
            .map_err(|_| anyhow!("peer reputation graph lock poisoned"))? = graph;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PeerRatingClass {
    Negative,
    Neutral,
    Positive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RatingPublication {
    subject: String,
    score: i64,
    class: PeerRatingClass,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PublishedPeerRating {
    score: i64,
    class: PeerRatingClass,
    published_at_ms: u64,
}

#[derive(Debug, Default)]
pub(super) struct PeerRatingPublisher {
    published: BTreeMap<String, PublishedPeerRating>,
}

impl PeerRatingPublisher {
    pub(super) fn from_events(events: &[Event], local_root: &str) -> Self {
        let mut publisher = Self::default();
        for event in events {
            if event.pubkey.to_hex() != local_root {
                continue;
            }
            let Ok(rating) = rating_from_event(event) else {
                continue;
            };
            let Some(publication) = Self::publication(&rating) else {
                continue;
            };
            let published_at_ms = rating.created_at.saturating_mul(1_000);
            if publisher
                .published
                .get(&publication.subject)
                .is_some_and(|previous| previous.published_at_ms >= published_at_ms)
            {
                continue;
            }
            publisher.record(publication, published_at_ms);
        }
        publisher
    }

    fn publication(rating: &Rating) -> Option<RatingPublication> {
        if rating.scope.as_deref() != Some(DEFAULT_PEER_RATING_SCOPE) {
            return None;
        }
        let subject = PublicKey::parse(&rating.subject).ok()?.to_hex();
        let score = rating.normalized_score().ok()?;
        let class = match score.cmp(&0) {
            std::cmp::Ordering::Less => PeerRatingClass::Negative,
            std::cmp::Ordering::Equal => PeerRatingClass::Neutral,
            std::cmp::Ordering::Greater => PeerRatingClass::Positive,
        };
        Some(RatingPublication {
            subject,
            score,
            class,
        })
    }

    pub(super) fn candidate(&self, rating: &Rating, now_ms: u64) -> Option<RatingPublication> {
        let candidate = Self::publication(rating)?;
        if candidate.class != PeerRatingClass::Negative
            && rating.sample_count.unwrap_or(0) < MIN_POSITIVE_RATING_SAMPLES
        {
            return None;
        }
        let Some(previous) = self.published.get(&candidate.subject) else {
            return Some(candidate);
        };
        let elapsed = now_ms.saturating_sub(previous.published_at_ms);
        if candidate.class == PeerRatingClass::Negative
            && previous.class != PeerRatingClass::Negative
        {
            return Some(candidate);
        }
        if elapsed >= RATING_REFRESH_INTERVAL_MS
            || (elapsed >= RATING_MIN_PUBLISH_INTERVAL_MS
                && (candidate.class != previous.class
                    || candidate.score.abs_diff(previous.score)
                        >= RATING_MATERIAL_SCORE_DELTA as u64))
        {
            return Some(candidate);
        }
        None
    }

    pub(super) fn record(&mut self, publication: RatingPublication, now_ms: u64) {
        self.published.insert(
            publication.subject,
            PublishedPeerRating {
                score: publication.score,
                class: publication.class,
                published_at_ms: now_ms,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use nostr_sdk::prelude::{Keys, ToBech32};
    use nostr_social_memory::RatingEventExt;

    use super::*;

    #[test]
    fn default_reputation_rejects_forgery_and_replaces_latest_rating() {
        let root = Keys::generate();
        let peer = Keys::generate();
        let attacker = Keys::generate();
        let root_hex = root.public_key().to_hex();
        let peer_hex = peer.public_key().to_hex();
        let peer_npub = peer.public_key().to_bech32().expect("peer npub");
        let (mut reputation, policy) =
            DefaultPeerReputation::new(&root.public_key().to_bech32().expect("root npub"))
                .expect("default reputation");

        let forged = rating_event(&attacker, &root_hex, &peer_hex, 0, 1);
        assert!(!reputation.ingest_event(&forged).expect("forged rating"));
        assert!(
            policy
                .select_mesh_peer(&peer_npub)
                .expect("unknown peer decision")
                .expect("unknown peer remains eligible")
                .is_unknown()
        );

        let negative = rating_event(&root, &root_hex, &peer_hex, 0, 2);
        assert!(reputation.ingest_event(&negative).expect("negative rating"));
        assert_eq!(
            policy
                .select_mesh_peer(&peer_npub)
                .expect("negative peer decision"),
            None
        );

        let recovered = rating_event(&root, &root_hex, &peer_hex, 100, 3);
        assert!(
            reputation
                .ingest_event(&recovered)
                .expect("recovery rating")
        );
        assert!(
            policy
                .select_mesh_peer(&peer_npub)
                .expect("recovered peer decision")
                .expect("recovered peer remains eligible")
                .quality_score
                .is_some_and(|score| score > 0)
        );

        assert!(!reputation.ingest_event(&negative).expect("stale rating"));
        assert!(
            policy
                .select_mesh_peer(&peer_npub)
                .expect("stale rating decision")
                .expect("stale rating cannot remove peer")
                .quality_score
                .is_some_and(|score| score > 0)
        );
    }

    #[test]
    fn rating_publisher_coalesces_changes_and_refreshes_daily() {
        let subject = Keys::generate().public_key().to_hex();
        let mut publisher = PeerRatingPublisher::default();
        let mut rating = Rating::new("rater", &subject, 80, 0, 100);
        rating.scope = Some(DEFAULT_PEER_RATING_SCOPE.to_string());
        rating.sample_count = Some(MIN_POSITIVE_RATING_SAMPLES);

        let first = publisher
            .candidate(&rating, 1_000)
            .expect("first evidenced rating publishes");
        publisher.record(first, 1_000);

        rating.rating = 85;
        assert!(publisher.candidate(&rating, 2_000).is_none());
        rating.rating = 95;
        assert!(publisher.candidate(&rating, 2_000).is_none());
        let material = publisher
            .candidate(&rating, 1_000 + RATING_MIN_PUBLISH_INTERVAL_MS)
            .expect("material change publishes after the minimum interval");
        publisher.record(material, 1_000 + RATING_MIN_PUBLISH_INTERVAL_MS);

        rating.rating = 0;
        let negative = publisher
            .candidate(&rating, 1_001 + RATING_MIN_PUBLISH_INTERVAL_MS)
            .expect("newly negative rating publishes immediately");
        publisher.record(negative, 1_001 + RATING_MIN_PUBLISH_INTERVAL_MS);
        assert!(
            publisher
                .candidate(&rating, 2_000 + RATING_MIN_PUBLISH_INTERVAL_MS)
                .is_none()
        );
        assert!(
            publisher
                .candidate(
                    &rating,
                    1_001 + RATING_MIN_PUBLISH_INTERVAL_MS + RATING_REFRESH_INTERVAL_MS,
                )
                .is_some()
        );
    }

    fn rating_event(
        signer: &Keys,
        rater: &str,
        subject: &str,
        value: i64,
        created_at: u64,
    ) -> Event {
        let mut rating = Rating::new(rater, subject, value, 0, 100);
        rating.scope = Some(DEFAULT_PEER_RATING_SCOPE.to_string());
        rating.created_at = created_at;
        rating.to_event(signer).expect("signed rating")
    }
}
