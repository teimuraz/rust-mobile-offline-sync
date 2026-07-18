//! `ItemStorage` — the item storage: it takes the (shared) event log as a
//! dependency and owns the projection table.
//!
//! **One implementation, used by both the mobile SDK and the backend** — for the
//! demo they can literally share it because both are in-memory. In a real app the
//! *implementations* are separate — SQLite behind the mobile SDK, Postgres (or
//! MySQL, …) behind the backend — but they keep this same shape: store = append
//! events + rebuild projection; reads = query the projection table.
//!
//! - **Write path:** `store` drains the entity's uncommitted events, appends them
//!   to the event log (the source of truth), then rebuilds that entity's row in
//!   the projection table.
//! - **Read path:** `find` / `find_by_id` never touch the event log — they query
//!   the projection table directly.
//! - **`build_projection` is public** so the sync side can rebuild entities after
//!   pulling/receiving remote events.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use uuid::Uuid;

use crate::core::EventSourcedEntity;
use std::sync::Arc as StdArc;
use crate::event_log::{AppendEvent, EntityEventLog};
use crate::item::{Item, ItemEvent, ItemId};
use crate::replica::ReplicaIdProvider;

/// The item event log as the storage sees it — device log on mobile, server log
/// on the backend.
pub type EntityLogHandle = StdArc<Mutex<dyn EntityEventLog<ItemEvent, ItemId>>>;

/// The item storage, shared within one replica by services and the sync side
/// (projection rebuilds).
pub type ItemStorageHandle = Arc<Mutex<ItemStorage>>;

/// Query filter for the projection table. One `find(filter)` builds the query;
/// convenience finders construct a filter and delegate to it.
#[derive(Default)]
pub struct ItemFilter {
    pub name: Option<String>,
}

impl ItemFilter {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn with_name(name: impl Into<String>) -> Self {
        Self {
            name: Some(name.into()),
        }
    }
}

pub struct ItemStorage {
    /// The shared event log (source of truth). Also used directly by the sync
    /// side — the storage is one consumer of it, not its owner.
    log: EntityLogHandle,
    /// Stamps `replica_id` onto locally-authored events at store time (the domain
    /// layer never sees provenance).
    replica_id_provider: ReplicaIdProvider,
    /// The `items` projection table — in-memory stand-in for a database table.
    /// Deleted items have no row here — deletion removes the row.
    items: HashMap<ItemId, Item>,
}

impl ItemStorage {
    pub fn new(log: EntityLogHandle, replica_id_provider: ReplicaIdProvider) -> Self {
        Self {
            log,
            replica_id_provider,
            items: HashMap::new(),
        }
    }

    /// Persist an entity: drain its uncommitted events into the event log, then
    /// rebuild its projection row from history. Provenance is stamped here — the
    /// replica id from the provider; the write offset by the log on append
    /// (`write_offset: None` = locally authored, assign one).
    pub async fn store(&mut self, item: &mut Item) {
        let entity_id = item.id().clone();
        let replica_id = self.replica_id_provider.get();
        let append: Vec<AppendEvent<ItemEvent, ItemId>> = item
            .uncommitted_events()
            .drain(..)
            .map(|e| AppendEvent {
                replica_event_id: Uuid::new_v4(),
                entity_id: entity_id.clone(),
                payload: e.event,
                replica_id: replica_id.clone(),
                replica_time_ms: e.modified_at_ms,
                replica_write_offset: None,
                server_offset: None,
                server_time_ms: None,
            })
            .collect();
        self.log.lock().await.append_local(append).await;
        self.build_projection(&entity_id).await;
    }

    /// Re-fold one entity from its event history into the projection table.
    /// A deletion event removes the row — deletion wins over concurrent edits.
    pub async fn build_projection(&mut self, id: &ItemId) {
        let events = self.log.lock().await.events_of_entity(id).await;
        if events.is_empty() {
            self.items.remove(id);
            return;
        }
        let item = Item::build_from_history(events);
        if item.deleted {
            self.items.remove(id);
        } else {
            self.items.insert(id.clone(), item);
        }
    }

    /// The one query method — reads the projection table, never the log.
    pub async fn find(&self, filter: &ItemFilter) -> Vec<Item> {
        let mut items: Vec<Item> = self
            .items
            .values()
            .filter(|it| match &filter.name {
                Some(name) => it.name.eq_ignore_ascii_case(name),
                None => true,
            })
            .cloned()
            .collect();
        items.sort_by(|a, b| a.name.cmp(&b.name));
        items
    }

    pub async fn find_by_id(&self, id: &ItemId) -> Option<Item> {
        self.items.get(id).cloned()
    }

    pub async fn find_all(&self) -> Vec<Item> {
        self.find(&ItemFilter::empty()).await
    }
}
