//! The **server-side** event log — the analog of the real backend `SqlEventLog`
//! (Postgres in the real app; in-memory here).
//!
//! `append` receives events pushed by devices (replica provenance already set,
//! preserved) and assigns the server's own provenance: a monotonic
//! `server_offset` and `server_time_ms`. Idempotent: an already-present event is
//! not stored twice but IS returned, so a device retrying a push after a lost
//! acknowledgement still learns the assigned offset. `events_after` is the pull
//! primitive: the stream in `server_offset` order.

use std::sync::Arc;

use shared::{
    sort_events, AppendEvent, EntityEventLog, EventDescriptor, EventsPage, ItemEvent, ItemId,
    ServerOffset,
};
use tokio::sync::Mutex;

/// Handle to the server's item event log.
pub type ServerLogHandle = Arc<Mutex<ServerEventLog>>;

#[derive(Default)]
pub struct ServerEventLog {
    entries: Vec<EventDescriptor<ItemEvent, ItemId>>,
    next_server_offset: u64,
}

impl ServerEventLog {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            next_server_offset: 1,
        }
    }

    fn find(&self, id: &uuid::Uuid) -> Option<&EventDescriptor<ItemEvent, ItemId>> {
        self.entries.iter().find(|d| &d.replica_event_id == id)
    }

    fn assign(&mut self, mut ev: AppendEvent<ItemEvent, ItemId>, batch_write_offset: u64) -> EventDescriptor<ItemEvent, ItemId> {
        let server_offset = ServerOffset(self.next_server_offset);
        self.next_server_offset += 1;
        let descriptor = EventDescriptor {
            replica_event_id: ev.replica_event_id,
            replica_write_offset: ev.replica_write_offset.take().unwrap_or(batch_write_offset),
            replica_time_ms: ev.replica_time_ms,
            replica_id: ev.replica_id,
            server_offset: Some(server_offset),
            server_time_ms: Some(now_ms()),
            entity_id: ev.entity_id,
            payload: ev.payload,
        };
        self.entries.push(descriptor.clone());
        descriptor
    }

    /// Store pushed events, preserving replica provenance and assigning
    /// `server_offset`/`server_time_ms`. Returns the events as stored.
    pub async fn append(
        &mut self,
        events: Vec<AppendEvent<ItemEvent, ItemId>>,
    ) -> Vec<EventDescriptor<ItemEvent, ItemId>> {
        let mut stored = Vec::new();
        for ev in events {
            if let Some(existing) = self.find(&ev.replica_event_id) {
                stored.push(existing.clone());
                continue;
            }
            let d = self.assign(ev, 0);
            stored.push(d);
        }
        stored
    }

    /// Every stored event in `server_offset` order — demo inspection endpoint.
    pub async fn all_events(&self) -> Vec<EventDescriptor<ItemEvent, ItemId>> {
        let mut out = self.entries.clone();
        out.sort_by_key(|d| d.server_offset);
        out
    }

    /// The pull primitive: events after `offset` in `server_offset` order.
    pub async fn events_after(
        &self,
        offset: Option<ServerOffset>,
        limit: usize,
    ) -> EventsPage<ItemEvent, ItemId> {
        let after = offset.map(|o| o.0).unwrap_or(0);
        let mut page: Vec<_> = self
            .entries
            .iter()
            .filter(|d| d.server_offset.map(|o| o.0 > after).unwrap_or(false))
            .cloned()
            .collect();
        page.sort_by_key(|d| d.server_offset);
        page.truncate(limit);
        let next_offset = page.last().and_then(|d| d.server_offset);
        EventsPage {
            events: page,
            next_offset,
        }
    }
}

#[async_trait::async_trait]
impl EntityEventLog<ItemEvent, ItemId> for ServerEventLog {
    /// Server-side authoring (e.g. admin edits): assigns both replica and server
    /// provenance — the server is a replica too.
    async fn append_local(
        &mut self,
        events: Vec<AppendEvent<ItemEvent, ItemId>>,
    ) -> Vec<EventDescriptor<ItemEvent, ItemId>> {
        let mut stored = Vec::new();
        let mut batch_write_offset = 0;
        for ev in events {
            if let Some(existing) = self.find(&ev.replica_event_id) {
                stored.push(existing.clone());
                continue;
            }
            batch_write_offset += 1;
            let d = self.assign(ev, batch_write_offset);
            stored.push(d);
        }
        stored
    }

    async fn events_of_entity(&self, entity_id: &ItemId) -> Vec<EventDescriptor<ItemEvent, ItemId>> {
        let mut out: Vec<_> = self
            .entries
            .iter()
            .filter(|d| &d.entity_id == entity_id)
            .cloned()
            .collect();
        sort_events(&mut out);
        out
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
