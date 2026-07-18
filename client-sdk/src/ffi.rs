//! The FFI layer — thin
//! objects that delegate to the pure Rust service and map domain types to
//! FFI-friendly records. No logic lives here. (`SyncRunner` has its own module.)
//!
//! Exported async: uniffi bridges these to Swift `async` functions, running them
//! on the embedded tokio runtime.

use std::sync::Arc;

use shared::{
    Item, ItemId, ItemStorage, ReplicaIdProvider, EntityLogHandle, ItemStorageHandle,
};

use crate::event_log::{DeviceEventLog, DeviceLogHandle};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::item_service::SdkItemService;

/// Constructs and wires the shared components — the event log, the storage (which
/// depends on the log), and the replica identity. The demo-scale analog of the
/// real SDK's `SdkEventSourcedStores`: build it once, then hand it to the item
/// service and the sync runner so they operate on the *same* log and storage.
#[derive(uniffi::Object)]
pub struct EventSourcedStores {
    pub(crate) replica_id: String,
    pub(crate) item_log: DeviceLogHandle,
    pub(crate) item_storage: ItemStorageHandle,
}

#[uniffi::export]
impl EventSourcedStores {
    /// `replica_id` identifies this device in event provenance
    #[uniffi::constructor]
    pub fn new(replica_id: String) -> Arc<Self> {
        let item_log: DeviceLogHandle = Arc::new(Mutex::new(DeviceEventLog::new()));
        let storage_log: EntityLogHandle = item_log.clone();
        let item_storage: ItemStorageHandle = Arc::new(Mutex::new(ItemStorage::new(
            storage_log,
            ReplicaIdProvider::new(replica_id.clone()),
        )));
        Arc::new(Self {
            replica_id,
            item_log,
            item_storage,
        })
    }
}

/// A flattened item for the UI. (The domain `Item` carries event-sourcing state
/// that doesn't belong across the FFI boundary, so we map it to plain fields.)
#[derive(uniffi::Record, Clone)]
pub struct ItemView {
    pub id: String,
    pub name: String,
    pub quantity: i64,
    pub note: Option<String>,
}

impl From<Item> for ItemView {
    fn from(it: Item) -> Self {
        ItemView {
            id: it.id.0.to_string(),
            name: it.name,
            quantity: it.quantity,
            note: it.note,
        }
    }
}

/// The item service as seen from Swift/Kotlin. Local-only: every operation works
/// fully offline. Syncing is the `SyncRunner`'s job, which shares the storage via
/// `EventSourcedStores`.
#[derive(uniffi::Object)]
pub struct ItemService {
    inner: SdkItemService,
}

#[uniffi::export(async_runtime = "tokio")]
impl ItemService {
    #[uniffi::constructor]
    pub fn new(stores: Arc<EventSourcedStores>) -> Arc<Self> {
        Arc::new(Self {
            inner: SdkItemService::new(stores.item_storage.clone()),
        })
    }

    /// Create an item locally (offline). Returns its id.
    pub async fn create_item(&self, name: String, quantity: i64) -> String {
        self.inner.create_item(name, quantity).await.0.to_string()
    }

    pub async fn set_quantity(&self, id: String, quantity: i64) {
        if let Some(id) = parse_id(&id) {
            self.inner.set_quantity(&id, quantity).await;
        }
    }

    pub async fn rename(&self, id: String, name: String) {
        if let Some(id) = parse_id(&id) {
            self.inner.rename(&id, name).await;
        }
    }

    pub async fn set_note(&self, id: String, note: Option<String>) {
        if let Some(id) = parse_id(&id) {
            self.inner.set_note(&id, note).await;
        }
    }

    pub async fn delete(&self, id: String) {
        if let Some(id) = parse_id(&id) {
            self.inner.delete(&id).await;
        }
    }

    /// Current items, from the projection table.
    pub async fn items(&self) -> Vec<ItemView> {
        self.inner
            .items()
            .await
            .into_iter()
            .map(ItemView::from)
            .collect()
    }
}

fn parse_id(s: &str) -> Option<ItemId> {
    Uuid::parse_str(s).ok().map(ItemId)
}
