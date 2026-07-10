use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use nostr_pubsub_social_graph::{MeshPeerPolicy, SocialGraphPolicy, SocialGraphPolicyConfig};
use nostr_sdk::prelude::{Event, Keys};
use nostr_social_graph::{Rating, RatingGraphConfig, SocialGraph};
use nostr_social_memory::{RatingEventExt, rating_from_event};

use crate::{NodeSpec, SimulationConfig};

pub(crate) const PEER_RATING_SCOPE: &str = "fips.peer";
const ATTACKER_EXPLORATION_INTERVAL: usize = 4;

pub(crate) type SharedMeshPeerPolicy = Arc<dyn MeshPeerPolicy>;
pub(crate) type SharedReputationGraph = Arc<RwLock<SocialGraph>>;

#[derive(Debug, Default)]
pub(crate) struct ReputationSeedReport {
    pub(crate) trusted_positive_ratings_applied: usize,
    pub(crate) trusted_negative_ratings_applied: usize,
    pub(crate) untrusted_ratings_ignored: usize,
    pub(crate) unknown_honest_peer_links: usize,
    pub(crate) unknown_attacker_peer_links: usize,
}

pub(crate) struct ReputationSetup {
    pub(crate) policies: Vec<Option<SharedMeshPeerPolicy>>,
    pub(crate) graphs: Vec<Option<SharedReputationGraph>>,
    pub(crate) report: ReputationSeedReport,
}

pub(crate) fn build_reputation_policies(
    config: &SimulationConfig,
    specs: &[NodeSpec],
    keys: &[Keys],
) -> Result<ReputationSetup> {
    let honest_count = config.node_count - config.attacker_count;
    let rating_config = RatingGraphConfig::for_scopes([PEER_RATING_SCOPE]);
    let mut policies = Vec::with_capacity(config.node_count);
    let mut graphs = Vec::with_capacity(config.node_count);
    let mut report = ReputationSeedReport::default();

    for index in 0..config.node_count {
        if index >= honest_count {
            policies.push(None);
            graphs.push(None);
            continue;
        }

        let root = keys[index].public_key().to_hex();
        let first_honest_unknown = specs[index]
            .neighbors
            .iter()
            .copied()
            .find(|neighbor| *neighbor < honest_count);
        let first_attacker_neighbor = specs[index]
            .neighbors
            .iter()
            .copied()
            .find(|neighbor| *neighbor >= honest_count);
        let first_attacker_unknown = if index % ATTACKER_EXPLORATION_INTERVAL == 0 {
            first_attacker_neighbor
        } else {
            None
        };
        let mut ratings = Vec::new();

        for neighbor in specs[index].neighbors.iter().copied() {
            if neighbor < honest_count && Some(neighbor) == first_honest_unknown {
                report.unknown_honest_peer_links += 1;
                continue;
            }
            if neighbor >= honest_count && Some(neighbor) == first_attacker_unknown {
                report.unknown_attacker_peer_links += 1;
                continue;
            }
            let value = if neighbor < honest_count { 100 } else { 0 };
            ratings.push(canonical_rating(
                &keys[index],
                &root,
                &keys[neighbor].public_key().to_hex(),
                value,
                1_000 + neighbor as u64,
                if value > 0 {
                    "valid local peer behavior"
                } else {
                    "repeated unanswered inventories"
                },
            )?);
        }

        if let Some(attacker) = first_attacker_neighbor {
            let attacker_pubkey = keys[attacker].public_key().to_hex();
            ratings.push(canonical_rating(
                &keys[attacker],
                &attacker_pubkey,
                &attacker_pubkey,
                100,
                10_000 + attacker as u64,
                "untrusted self-praise",
            )?);
            if let Some(honest_neighbor) = first_honest_unknown {
                ratings.push(canonical_rating(
                    &keys[attacker],
                    &attacker_pubkey,
                    &keys[honest_neighbor].public_key().to_hex(),
                    0,
                    20_000 + attacker as u64,
                    "untrusted false accusation",
                )?);
            }
        }

        let mut graph = SocialGraph::new(&root);
        let projection = graph.apply_ratings(&ratings, &rating_config)?;
        report.trusted_positive_ratings_applied = report
            .trusted_positive_ratings_applied
            .saturating_add(projection.positive_ratings);
        report.trusted_negative_ratings_applied = report
            .trusted_negative_ratings_applied
            .saturating_add(projection.negative_ratings);
        report.untrusted_ratings_ignored = report
            .untrusted_ratings_ignored
            .saturating_add(projection.ignored_ratings);

        let graph = Arc::new(RwLock::new(graph));
        let policy: SharedMeshPeerPolicy = Arc::new(SocialGraphPolicy::new(
            Arc::clone(&graph),
            SocialGraphPolicyConfig::default(),
        ));
        policies.push(Some(policy));
        graphs.push(Some(graph));
    }

    Ok(ReputationSetup {
        policies,
        graphs,
        report,
    })
}

pub(crate) fn canonical_rating_event(
    signer: &Keys,
    rater: &str,
    subject: &str,
    value: i64,
    created_at: u64,
    reason: &str,
) -> Result<Event> {
    let mut rating = Rating::new(rater, subject, value, 0, 100);
    rating.scope = Some(PEER_RATING_SCOPE.to_string());
    rating.sample_count = Some(1);
    rating.created_at = created_at;
    rating.reason = Some(reason.to_string());
    rating
        .to_event(signer)
        .context("failed to sign canonical peer rating")
}

fn canonical_rating(
    signer: &Keys,
    rater: &str,
    subject: &str,
    value: i64,
    created_at: u64,
    reason: &str,
) -> Result<Rating> {
    let event = canonical_rating_event(signer, rater, subject, value, created_at, reason)?;
    rating_from_event(&event).context("failed to parse canonical peer rating")
}
