#[derive(Debug, Default, Serialize, Deserialize)]
struct StoredEventsFile {
    version: u8,
    events: Vec<Event>,
}

#[derive(Debug)]
struct ControlEventStore {
    path: Option<PathBuf>,
    events: HashMap<String, Event>,
    order: VecDeque<String>,
    rating_events: HashMap<RatingEventStoreKey, RatingEventStoreEntry>,
    update_events: UpdateEventCache,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RatingEventStoreKey {
    author: String,
    subject: String,
    scope: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RatingEventStoreEntry {
    event_id: String,
    created_at: u64,
}

fn configured_update_events() -> Result<UpdateEventCache> {
    let reference = configured_update_ref()?;
    UpdateEventCache::new(&reference).context("failed to configure update announcement cache")
}

impl ControlEventStore {
    fn load(path: Option<PathBuf>, update_events: UpdateEventCache) -> Result<Self> {
        let Some(path) = path else {
            return Ok(Self {
                path: None,
                events: HashMap::new(),
                order: VecDeque::new(),
                rating_events: HashMap::new(),
                update_events,
            });
        };
        let mut store = Self {
            path: Some(path.clone()),
            events: HashMap::new(),
            order: VecDeque::new(),
            rating_events: HashMap::new(),
            update_events,
        };
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(store),
            Err(error) => {
                return Err(error).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        let saved: StoredEventsFile = serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to decode {}", path.display()))?;
        if saved.version != STORE_VERSION {
            return Err(anyhow!(
                "unsupported control pubsub store version {} in {}",
                saved.version,
                path.display()
            ));
        }
        let saved_count = saved.events.len();
        for event in saved.events {
            if event.verify().is_ok()
                && is_control_event(&event, &store.update_events)
                && control_event_is_persistent(&event)
            {
                let _ = store.insert_memory(event);
            }
        }
        if store.events.len() != saved_count {
            store.persist()?;
        }
        Ok(store)
    }

    fn insert(&mut self, event: Event) -> Result<bool> {
        let persistent = control_event_is_persistent(&event);
        if !self.insert_memory(event) {
            return Ok(false);
        }
        if persistent {
            self.persist()?;
        }
        Ok(true)
    }

    fn insert_memory(&mut self, event: Event) -> bool {
        let event_id = event.id.to_hex();
        if self.events.contains_key(&event_id) {
            return false;
        }
        let rating = if u16::from(event.kind) == RATING_FACT_KIND {
            let Some((rating_key, created_at)) = retained_rating_event(&event, now_ms() / 1_000)
            else {
                return false;
            };
            if let Some(stored) = self.rating_events.get(&rating_key).cloned() {
                let stored_event = self
                    .events
                    .get(&stored.event_id)
                    .expect("rating index refers to a stored event");
                if (stored.created_at, stored_event.id) >= (created_at, event.id) {
                    return false;
                }
                self.remove_memory(&stored.event_id);
            }
            Some((rating_key, created_at))
        } else {
            None
        };
        let is_update_event = self
            .update_events
            .filter()
            .match_event(&event, MatchEventOptions::new());
        if is_update_event {
            if !self.update_events.ingest_event(event.clone()).unwrap_or(false) {
                return false;
            }
            let replaced = self
                .events
                .iter()
                .filter(|(_, stored)| {
                    self.update_events
                        .filter()
                        .match_event(stored, MatchEventOptions::new())
                })
                .map(|(event_id, _)| event_id.clone())
                .collect::<Vec<_>>();
            for stored_id in replaced {
                self.remove_memory(&stored_id);
            }
        }
        while self.events.len() >= STORE_MAX_EVENTS {
            let remove_index = self
                .order
                .iter()
                .position(|stored_id| {
                    self.events
                        .get(stored_id)
                        .is_some_and(|stored| {
                            !self
                                .update_events
                                .filter()
                                .match_event(stored, MatchEventOptions::new())
                        })
                })
                .unwrap_or(0);
            let Some(oldest) = self.order.remove(remove_index) else {
                break;
            };
            self.remove_memory(&oldest);
        }
        self.order.push_back(event_id.clone());
        self.events.insert(event_id.clone(), event);
        if let Some((rating_key, created_at)) = rating {
            self.rating_events.insert(
                rating_key,
                RatingEventStoreEntry {
                    event_id,
                    created_at,
                },
            );
        }
        true
    }

    fn remove_memory(&mut self, event_id: &str) -> bool {
        if self.events.remove(event_id).is_none() {
            return false;
        }
        self.order.retain(|stored_id| stored_id != event_id);
        self.rating_events
            .retain(|_, stored| stored.event_id != event_id);
        true
    }

    fn snapshot(&self) -> Vec<Event> {
        self.order
            .iter()
            .filter_map(|event_id| self.events.get(event_id).cloned())
            .collect()
    }

    fn prune_expired_ratings(&mut self, now_secs: u64) -> Result<usize> {
        let remove = self
            .rating_events
            .iter()
            .filter(|(_, stored)| {
                now_secs.saturating_sub(stored.created_at) > PEER_RATING_MAX_AGE.as_secs()
            })
            .map(|(_, stored)| stored.event_id.clone())
            .collect::<Vec<_>>();
        if remove.is_empty() {
            return Ok(0);
        }
        for event_id in &remove {
            self.remove_memory(event_id);
        }
        self.persist()?;
        Ok(remove.len())
    }

    fn persist(&self) -> Result<()> {
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let saved = StoredEventsFile {
            version: STORE_VERSION,
            events: self
                .snapshot()
                .into_iter()
                .filter(control_event_is_persistent)
                .collect(),
        };
        let bytes = serde_json::to_vec(&saved).context("failed to encode control pubsub store")?;
        let temporary = temporary_store_path(path);
        fs::write(&temporary, bytes)
            .with_context(|| format!("failed to write {}", temporary.display()))?;
        fs::rename(&temporary, path).with_context(|| {
            format!(
                "failed to replace control pubsub store {} with {}",
                path.display(),
                temporary.display()
            )
        })?;
        Ok(())
    }
}

fn control_event_is_persistent(event: &Event) -> bool {
    u16::from(event.kind) != FIPS_PEER_ADVERT_KIND
}

fn retained_rating_event(event: &Event, now_secs: u64) -> Option<(RatingEventStoreKey, u64)> {
    let (key, created_at) = rating_event_store_key(event)?;
    if created_at > now_secs.saturating_add(PEER_RATING_MAX_FUTURE_SKEW.as_secs())
        || now_secs.saturating_sub(created_at) > PEER_RATING_MAX_AGE.as_secs()
    {
        return None;
    }
    Some((key, created_at))
}

fn rating_event_store_key(event: &Event) -> Option<(RatingEventStoreKey, u64)> {
    if u16::from(event.kind) != RATING_FACT_KIND {
        return None;
    }
    let rating = rating_from_event(event).ok()?;
    let subject = PublicKey::parse(&rating.subject).ok()?.to_hex();
    let scope = rating.scope?.trim().to_string();
    if scope.is_empty() {
        return None;
    }
    Some((
        RatingEventStoreKey {
            author: event.pubkey.to_hex(),
            subject,
            scope,
        },
        rating.created_at,
    ))
}

#[cfg(test)]
mod tests {
    use nostr_sdk::{EventBuilder, ToBech32};
    use nostr_social_graph::Rating;
    use nostr_social_memory::RatingEventExt;

    use super::*;

    #[test]
    fn rating_events_are_coalesced_by_author_subject_and_scope() {
        let author = Keys::generate();
        let subject = Keys::generate().public_key().to_hex();
        let update_events = test_update_events();
        let now = now_ms() / 1_000;
        let mut store = ControlEventStore::load(None, update_events).expect("event store");
        let older = rating_event(&author, &subject, "fips.peer", 20, now.saturating_sub(1));
        let newer = rating_event(&author, &subject, "fips.peer", 80, now);

        assert!(store.insert(older).expect("insert older rating"));
        assert!(store.insert(newer.clone()).expect("insert newer rating"));
        assert_eq!(store.snapshot(), vec![newer]);
        assert_eq!(store.rating_events.len(), 1);
    }

    #[test]
    fn stale_or_far_future_ratings_are_not_retained() {
        let author = Keys::generate();
        let subject = Keys::generate().public_key().to_hex();
        let update_events = test_update_events();
        let now = now_ms() / 1_000;
        let mut store = ControlEventStore::load(None, update_events).expect("event store");
        let stale = rating_event(
            &author,
            &subject,
            "fips.peer",
            20,
            now.saturating_sub(PEER_RATING_MAX_AGE.as_secs() + 1),
        );
        let future = rating_event(
            &author,
            &subject,
            "fips.peer",
            80,
            now.saturating_add(PEER_RATING_MAX_FUTURE_SKEW.as_secs() + 60),
        );

        assert!(!store.insert(stale).expect("reject stale rating"));
        assert!(!store.insert(future).expect("reject future rating"));
        assert!(store.snapshot().is_empty());
    }

    #[test]
    fn maintenance_prunes_ratings_that_age_out() {
        let author = Keys::generate();
        let subject = Keys::generate().public_key().to_hex();
        let update_events = test_update_events();
        let created_at = now_ms() / 1_000;
        let mut store = ControlEventStore::load(None, update_events).expect("event store");
        let rating = rating_event(&author, &subject, "fips.peer", 20, created_at);

        assert!(store.insert(rating).expect("insert rating"));
        assert_eq!(
            store
                .prune_expired_ratings(created_at + PEER_RATING_MAX_AGE.as_secs() + 1)
                .expect("prune ratings"),
            1
        );
        assert!(store.snapshot().is_empty());
    }

    #[test]
    fn peer_adverts_remain_in_memory_but_are_not_persisted() {
        let keys = Keys::generate();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "nvpn-control-event-store-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).expect("event store directory");
        let path = directory.join("control-events.json");
        let update_events = test_update_events();
        let mut store =
            ControlEventStore::load(Some(path.clone()), update_events).expect("event store");
        let advert = EventBuilder::new(Kind::Custom(FIPS_PEER_ADVERT_KIND), "")
            .sign_with_keys(&keys)
            .expect("signed peer advert");

        assert!(store.insert(advert.clone()).expect("insert peer advert"));
        assert_eq!(store.snapshot(), vec![advert.clone()]);
        assert!(!path.exists(), "ephemeral peer advert must not hit disk");

        let rating = rating_event(
            &keys,
            &Keys::generate().public_key().to_hex(),
            "fips.peer",
            50,
            now_ms() / 1_000,
        );
        assert!(store.insert(rating.clone()).expect("insert rating"));
        let saved: StoredEventsFile =
            serde_json::from_slice(&fs::read(&path).expect("persisted store"))
                .expect("decode persisted store");
        assert_eq!(saved.events, vec![rating.clone()]);

        let legacy = StoredEventsFile {
            version: STORE_VERSION,
            events: vec![advert, rating.clone()],
        };
        fs::write(&path, serde_json::to_vec(&legacy).expect("encode legacy store"))
            .expect("write legacy store");
        let reloaded = ControlEventStore::load(Some(path.clone()), test_update_events())
            .expect("reload event store");
        assert_eq!(reloaded.snapshot(), vec![rating.clone()]);
        let cleaned: StoredEventsFile =
            serde_json::from_slice(&fs::read(path).expect("cleaned store"))
                .expect("decode cleaned store");
        assert_eq!(cleaned.events, vec![rating]);
        fs::remove_dir_all(directory).expect("remove event store directory");
    }

    fn test_update_events() -> UpdateEventCache {
        let keys = Keys::generate();
        let reference = nostr_vpn_core::updater::UpdateRef {
            npub: keys.public_key().to_bech32().expect("npub"),
            tree_name: "test-root".to_string(),
            path: Some("latest".to_string()),
        };
        UpdateEventCache::new(&reference).expect("update event cache")
    }

    fn rating_event(
        author: &Keys,
        subject: &str,
        scope: &str,
        value: i64,
        created_at: u64,
    ) -> Event {
        let mut rating = Rating::new(author.public_key().to_hex(), subject, value, 0, 100);
        rating.scope = Some(scope.to_string());
        rating.created_at = created_at;
        rating.to_event(author).expect("signed rating")
    }
}
