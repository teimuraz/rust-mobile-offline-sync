//! Wire/domain types for event logs, plus the minimal contract `ItemStorage`
//! needs. The actual log *implementations* live on each side, like in the real
//! code: the device log in `client-sdk` (SQLite in reality), the server log in
//! `backend` (Postgres in reality).

use serde::{Deserialize, Serialize};

use crate::core::{EntityId, Event, EventDescriptor, ServerOffset};

/// An event ready to be appended to a log.
///
/// For `append_local` (locally authored): `replica_write_offset` and the server
/// fields are `None` — the local log assigns the write offset; the server assigns
/// `server_offset`/`server_time_ms` when the event reaches it. For replicated
/// appends, everything is `Some` and must be preserved.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppendEvent<E, EntId> {
    pub replica_event_id: uuid::Uuid,
    pub entity_id: EntId,
    pub payload: E,
    pub replica_id: String,
    pub replica_time_ms: i64,
    pub replica_write_offset: Option<u64>,
    pub server_offset: Option<ServerOffset>,
    pub server_time_ms: Option<i64>,
}

/// Turn a stored event back into an appendable one (used when shipping events
/// during sync). Provenance is `Some` — the receiving log must preserve it.
impl<E: Clone, EntId: Clone> From<&EventDescriptor<E, EntId>> for AppendEvent<E, EntId> {
    fn from(d: &EventDescriptor<E, EntId>) -> Self {
        AppendEvent {
            replica_event_id: d.replica_event_id,
            entity_id: d.entity_id.clone(),
            payload: d.payload.clone(),
            replica_id: d.replica_id.clone(),
            replica_time_ms: d.replica_time_ms,
            replica_write_offset: Some(d.replica_write_offset),
            server_offset: d.server_offset,
            server_time_ms: d.server_time_ms,
        }
    }
}

/// A page of events pulled from a log during sync, plus the cursor to resume from.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventsPage<E, EntId> {
    pub events: Vec<EventDescriptor<E, EntId>>,
    pub next_offset: Option<ServerOffset>,
}

/// The subset of an event log that `ItemStorage` needs. Implemented by both the
/// device log (`client-sdk`) and the server log (`backend`).
#[async_trait::async_trait]
pub trait EntityEventLog<E: Event, EntId: EntityId>: Send + Sync {
    /// Append locally-authored events, assigning local provenance.
    async fn append_local(
        &mut self,
        events: Vec<AppendEvent<E, EntId>>,
    ) -> Vec<EventDescriptor<E, EntId>>;

    /// Every event for one entity, in canonical fold order.
    async fn events_of_entity(&self, entity_id: &EntId) -> Vec<EventDescriptor<E, EntId>>;
}
