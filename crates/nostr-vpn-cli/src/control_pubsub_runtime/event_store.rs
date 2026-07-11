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
}

#[derive(Clone, Debug)]
struct UpdateRootSubscription {
    author: PublicKey,
    tree_name: String,
}

impl UpdateRootSubscription {
    fn configured() -> Result<Self> {
        let reference = configured_update_ref()?;
        let author = PublicKey::parse(&reference.npub)
            .with_context(|| format!("invalid update publisher {}", reference.npub))?;
        Ok(Self {
            author,
            tree_name: reference.tree_name,
        })
    }

    fn filter(&self) -> Filter {
        Filter::new()
            .kinds([
                Kind::Custom(HASHTREE_ROOT_KIND),
                Kind::Custom(HASHTREE_LEGACY_ROOT_KIND),
            ])
            .author(self.author)
            .custom_tag(
                SingleLetterTag::lowercase(Alphabet::D),
                self.tree_name.clone(),
            )
    }

    fn matches(&self, event: &Event) -> bool {
        matches!(
            u16::from(event.kind),
            HASHTREE_ROOT_KIND | HASHTREE_LEGACY_ROOT_KIND
        ) && event.pubkey == self.author
            && event.tags.iter().any(|tag| {
                matches!(
                    tag.as_standardized(),
                    Some(TagStandard::Identifier(identifier)) if identifier == &self.tree_name
                )
            })
    }
}

impl ControlEventStore {
    fn load(path: Option<PathBuf>, update_root: &UpdateRootSubscription) -> Result<Self> {
        let Some(path) = path else {
            return Ok(Self {
                path: None,
                events: HashMap::new(),
                order: VecDeque::new(),
            });
        };
        let mut store = Self {
            path: Some(path.clone()),
            events: HashMap::new(),
            order: VecDeque::new(),
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
        for event in saved.events {
            if event.verify().is_ok() && is_control_event(&event, update_root) {
                let _ = store.insert_memory(event);
            }
        }
        Ok(store)
    }

    fn insert(&mut self, event: Event) -> Result<bool> {
        if !self.insert_memory(event) {
            return Ok(false);
        }
        self.persist()?;
        Ok(true)
    }

    fn insert_memory(&mut self, event: Event) -> bool {
        let event_id = event.id.to_hex();
        if self.events.contains_key(&event_id) {
            return false;
        }
        if is_update_root_kind(u16::from(event.kind)) {
            if self.events.values().any(|stored| {
                same_replaceable_update_root(stored, &event)
                    && (stored.created_at, stored.id) >= (event.created_at, event.id)
            }) {
                return false;
            }
            let replaced = self
                .events
                .iter()
                .filter(|(_, stored)| same_replaceable_update_root(stored, &event))
                .map(|(event_id, _)| event_id.clone())
                .collect::<std::collections::HashSet<_>>();
            self.events
                .retain(|stored_id, _| !replaced.contains(stored_id));
            self.order.retain(|stored_id| !replaced.contains(stored_id));
        }
        while self.events.len() >= STORE_MAX_EVENTS {
            let remove_index = self
                .order
                .iter()
                .position(|stored_id| {
                    self.events
                        .get(stored_id)
                        .is_some_and(|stored| !is_update_root_kind(u16::from(stored.kind)))
                })
                .unwrap_or(0);
            let Some(oldest) = self.order.remove(remove_index) else {
                break;
            };
            self.events.remove(&oldest);
        }
        self.order.push_back(event_id.clone());
        self.events.insert(event_id, event);
        true
    }

    fn snapshot(&self) -> Vec<Event> {
        self.order
            .iter()
            .filter_map(|event_id| self.events.get(event_id).cloned())
            .collect()
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
            events: self.snapshot(),
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
