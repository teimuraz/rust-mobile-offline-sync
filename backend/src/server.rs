//! Endpoints:
//!   POST /events                 — append a batch of events (the "push" side);
//!                                  returns them with the assigned server_offset
//!   GET  /events?after=&limit=   — events after a server offset (the "pull" side)
//!   GET  /items                  — current items, folded from the pushed events
//!   GET  /health
//!
//! Storage is in-memory so the demo needs no database; in reality the log and the
//! projection table live in Postgres (or MySQL, …).

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use shared::{
    AppendEvent, EventDescriptor, EventsPage, Item, ItemEvent, ItemId, ItemStorage,
    ReplicaIdProvider, ServerOffset, EntityLogHandle, ItemStorageHandle,
};
use tokio::sync::Mutex;

use crate::event_log::{ServerEventLog, ServerLogHandle};

#[derive(Clone)]
pub struct ServerState {
    pub item_log: ServerLogHandle,
    pub item_storage: ItemStorageHandle,
}

pub fn build_router() -> Router {
    let item_log: ServerLogHandle = Arc::new(Mutex::new(ServerEventLog::new()));
    let storage_log: EntityLogHandle = item_log.clone();
    let item_storage: ItemStorageHandle = Arc::new(Mutex::new(ItemStorage::new(
        storage_log,
        ReplicaIdProvider::new("server"),
    )));
    build_router_with(ServerState {
        item_log,
        item_storage,
    })
}

pub fn build_router_with(state: ServerState) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/events", post(append_events).get(get_events))
        .route("/events/all", get(get_all_events))
        .route("/items", get(get_items))
        .with_state(state)
}

/// Push side: store the events (assigning `server_offset`), rebuild the
/// projections of every entity the batch touched, and return the stored events —
/// the device records the assigned offsets as its sync acknowledgement.
async fn append_events(
    State(state): State<ServerState>,
    Json(events): Json<Vec<AppendEvent<ItemEvent, ItemId>>>,
) -> Json<Vec<EventDescriptor<ItemEvent, ItemId>>> {
    let stored = {
        let mut log = state.item_log.lock().await;
        log.append(events).await
    };
    let mut storage = state.item_storage.lock().await;
    for descriptor in &stored {
        storage.build_projection(&descriptor.entity_id).await;
    }
    Json(stored)
}

/// The full item event log with all provenance — handy for poking at the demo:
/// `curl localhost:4000/events/all | jq`.
async fn get_all_events(
    State(state): State<ServerState>,
) -> Json<Vec<EventDescriptor<ItemEvent, ItemId>>> {
    let log = state.item_log.lock().await;
    Json(log.all_events().await)
}

#[derive(Deserialize)]
struct AfterQuery {
    after: Option<u64>,
    limit: Option<usize>,
}

/// Pull side: events after a server offset.
async fn get_events(
    State(state): State<ServerState>,
    Query(q): Query<AfterQuery>,
) -> Json<EventsPage<ItemEvent, ItemId>> {
    let log = state.item_log.lock().await;
    let page = log
        .events_after(q.after.map(ServerOffset), q.limit.unwrap_or(500))
        .await;
    Json(page)
}

/// The current items as the server sees them — folded from events pushed by the
/// phones. `curl localhost:4000/items` after a sync shows the data arrived.
#[derive(Serialize)]
struct ItemDto {
    id: String,
    name: String,
    quantity: i64,
    note: Option<String>,
}

impl From<Item> for ItemDto {
    fn from(it: Item) -> Self {
        ItemDto {
            id: it.id.0.to_string(),
            name: it.name,
            quantity: it.quantity,
            note: it.note,
        }
    }
}

async fn get_items(State(state): State<ServerState>) -> Json<Vec<ItemDto>> {
    let storage = state.item_storage.lock().await;
    let items = storage.find_all().await;
    Json(items.into_iter().map(ItemDto::from).collect())
}
