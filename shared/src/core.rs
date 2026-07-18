//! The tiny event-sourcing core, shared verbatim by the backend and the mobile app.

use serde::{de::DeserializeOwned, Deserialize, Serialize};

/// Anything that can be an event: cloneable + serializable (it travels over the
/// wire and is persisted as JSON on both sides).
pub trait Event:
    Clone + Serialize + DeserializeOwned + std::fmt::Debug + Send + Sync + 'static
{
}

/// Anything that can identify an entity.
pub trait EntityId:
    Clone
    + Eq
    + std::hash::Hash
    + std::fmt::Debug
    + Serialize
    + DeserializeOwned
    + Send
    + Sync
    + 'static
{
}

/// A monotonic position in the **server's** event stream, assigned by the server
/// when it stores an event. On a device, an event whose `server_offset` is `None`
/// has not been synced yet — that is the "unsynced" marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ServerOffset(pub u64);

/// A locally-produced event that hasn't been persisted yet. It carries only *when*
/// the change happened — replica id and write offset are provenance, stamped later
/// by the storage (via the replica-id provider) and the event log respectively.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityEvent<E> {
    pub event: E,
    pub modified_at_ms: i64,
}

/// A persisted event — the unit that syncs between replicas.
///
/// Mirrors the real descriptor: replica provenance (`replica_*`), plus the
/// server-assigned `server_offset`/`server_time_ms`, which are `None` until the
/// server has stored the event. (The real one also carries `space_id`,
/// `owner_user_id`, and `created_by` for authorization scoping — deliberately
/// omitted here, the demo leaves authorization out.)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventDescriptor<E, EntId> {
    /// Unique across all replicas.
    pub replica_event_id: uuid::Uuid,
    /// Per-replica tie-breaker for events sharing a timestamp; assigned by the
    /// event log on local append.
    pub replica_write_offset: u64,
    /// The authoring replica's wall-clock at creation.
    pub replica_time_ms: i64,
    /// Which device/server produced the event.
    pub replica_id: String,
    /// Position in the server's stream; `None` on a device until synced.
    pub server_offset: Option<ServerOffset>,
    /// Server wall-clock when the server stored the event.
    pub server_time_ms: Option<i64>,
    pub entity_id: EntId,
    pub payload: E,
}

/// The total order used to fold events deterministically on every replica:
/// `replica_time_ms` → `replica_write_offset` → `replica_event_id`.
pub fn order_key<E, EntId>(d: &EventDescriptor<E, EntId>) -> (i64, u64, uuid::Uuid) {
    (d.replica_time_ms, d.replica_write_offset, d.replica_event_id)
}

/// Sort a batch of events into the canonical fold order (in place).
pub fn sort_events<E, EntId>(events: &mut [EventDescriptor<E, EntId>]) {
    events.sort_by_key(order_key);
}

/// The one trait a domain entity implements.
pub trait EventSourcedEntity: Default {
    type Evt: Event;
    type EntId: EntityId;

    fn uncommitted_events(&mut self) -> &mut Vec<EntityEvent<Self::Evt>>;

    /// The only thing an entity must define: given the current state, one event,
    /// and when it happened, produce the next state.
    fn apply_event(&mut self, event: Self::Evt, modified_at_ms: i64);

    fn id(&self) -> &Self::EntId;

    /// Rebuild an entity by folding its full history. `events` must already be in
    /// [`order_key`] order.
    fn build_from_history(events: Vec<EventDescriptor<Self::Evt, Self::EntId>>) -> Self {
        let mut entity = Self::default();
        for event in events {
            let at = event.replica_time_ms;
            entity.apply_event(event.payload, at);
        }
        entity
    }

    /// Record a new local change: remember it as uncommitted, and apply it now so
    /// the in-memory state reflects it immediately (optimistic/offline update).
    fn new_event(&mut self, event: EntityEvent<Self::Evt>) {
        self.uncommitted_events().push(event.clone());
        self.apply_event(event.event, event.modified_at_ms);
    }
}
