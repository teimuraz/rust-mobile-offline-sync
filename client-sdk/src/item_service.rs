//! `SdkItemService` — pure local item operations.
//! It knows nothing about the network or syncing: every
//! method works fully offline against the local storage. Syncing is a separate
//! concern, handled by the `SyncRunner`, which shares the same storage.
//!
//! Flow for every mutation: load the projection from storage → call the domain
//! method on the entity (records an event with its timestamp) → `storage.store`
//! (stamps replica id, appends to the event log — which assigns the write offset —
//! and rebuilds the projection). The service carries no provenance state at all.

use std::time::{SystemTime, UNIX_EPOCH};

use shared::{EventSourcedEntity, Item, ItemId, ItemStorageHandle};
use uuid::Uuid;

pub struct SdkItemService {
    storage: ItemStorageHandle,
}

impl SdkItemService {
    pub fn new(storage: ItemStorageHandle) -> Self {
        Self { storage }
    }

    pub async fn create_item(&self, name: String, quantity: i64) -> ItemId {
        let mut item = Item::create(ItemId(Uuid::new_v4()), name, quantity, now_ms());
        self.storage.lock().await.store(&mut item).await;
        item.id().clone()
    }

    pub async fn set_quantity(&self, id: &ItemId, quantity: i64) {
        let mut storage = self.storage.lock().await;
        if let Some(mut item) = storage.find_by_id(id).await {
            item.set_quantity(quantity, now_ms());
            storage.store(&mut item).await;
        }
    }

    pub async fn rename(&self, id: &ItemId, name: String) {
        let mut storage = self.storage.lock().await;
        if let Some(mut item) = storage.find_by_id(id).await {
            item.rename(name, now_ms());
            storage.store(&mut item).await;
        }
    }

    pub async fn set_note(&self, id: &ItemId, note: Option<String>) {
        let mut storage = self.storage.lock().await;
        if let Some(mut item) = storage.find_by_id(id).await {
            item.set_note(note, now_ms());
            storage.store(&mut item).await;
        }
    }

    pub async fn delete(&self, id: &ItemId) {
        let mut storage = self.storage.lock().await;
        if let Some(mut item) = storage.find_by_id(id).await {
            item.delete(now_ms());
            storage.store(&mut item).await;
        }
    }

    /// Current items, queried from the projection table.
    pub async fn items(&self) -> Vec<Item> {
        self.storage.lock().await.find_all().await
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
