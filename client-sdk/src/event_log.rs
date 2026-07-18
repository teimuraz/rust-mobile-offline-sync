//! The **device-side** event log — the analog of the real `SdkEventLog` /
//! `SdkSqlEventLog` (SQLite in the real app; in-memory here).
//!
//! Two append paths, like the real code:
//! - `append_local` — events this device just authored: assigns
//!   `replica_write_offset`, leaves `server_offset` NULL (= unsynced).
//! - `append` — events replicated from the server (pull): all provenance is
//!   preserved, nothing is assigned.

use std::sync::Arc;

use shared::{
    sort_events, AppendEvent, EntityEventLog, EventDescriptor, ItemEvent, ItemId, ServerOffset,
};
use tokio::sync::Mutex;

/// Handle to this device's item event log.
pub type DeviceLogHandle = Arc<Mutex<DeviceEventLog>>;

#[derive(Default)]
pub struct DeviceEventLog {
    entries: Vec<EventDescriptor<ItemEvent, ItemId>>,
}

impl DeviceEventLog {
    pub fn new() -> Self {
        Self::default()
    }

    fn find(&self, id: &uuid::Uuid) -> Option<&EventDescriptor<ItemEvent, ItemId>> {
        self.entries.iter().find(|d| &d.replica_event_id == id)
    }

    /// Replicated append (pull side): preserve all provenance; idempotent.
    pub async fn append(&mut self, events: Vec<AppendEvent<ItemEvent, ItemId>>) {
        for ev in events {
            if self.find(&ev.replica_event_id).is_some() {
                continue;
            }
            self.entries.push(EventDescriptor {
                replica_event_id: ev.replica_event_id,
                replica_write_offset: ev.replica_write_offset.unwrap_or(0),
                replica_time_ms: ev.replica_time_ms,
                replica_id: ev.replica_id,
                server_offset: ev.server_offset,
                server_time_ms: ev.server_time_ms,
                entity_id: ev.entity_id,
                payload: ev.payload,
            });
        }
    }

    /// Events authored by `replica_id` with `server_offset IS NULL` — not yet
    /// acknowledged by the server. This is what push sends.
    pub async fn get_unsynced(
        &self,
        replica_id: &str,
        limit: usize,
    ) -> Vec<EventDescriptor<ItemEvent, ItemId>> {
        self.entries
            .iter()
            .filter(|d| d.server_offset.is_none() && d.replica_id == replica_id)
            .take(limit)
            .cloned()
            .collect()
    }

    /// Record the server's acknowledgement: stamp the assigned
    /// `server_offset`/`server_time_ms`, marking events synced.
    pub async fn mark_synced(&mut self, acknowledged: &[EventDescriptor<ItemEvent, ItemId>]) {
        for ack in acknowledged {
            if let Some(entry) = self
                .entries
                .iter_mut()
                .find(|d| d.replica_event_id == ack.replica_event_id)
            {
                entry.server_offset = ack.server_offset;
                entry.server_time_ms = ack.server_time_ms;
            }
        }
    }
}

#[async_trait::async_trait]
impl EntityEventLog<ItemEvent, ItemId> for DeviceEventLog {
    /// Local append: assign `replica_write_offset`; `server_offset` stays NULL
    /// until the server acknowledges the event.
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
            let descriptor = EventDescriptor {
                replica_event_id: ev.replica_event_id,
                replica_write_offset: ev.replica_write_offset.unwrap_or(batch_write_offset),
                replica_time_ms: ev.replica_time_ms,
                replica_id: ev.replica_id,
                server_offset: None,
                server_time_ms: None,
                entity_id: ev.entity_id,
                payload: ev.payload,
            };
            self.entries.push(descriptor.clone());
            stored.push(descriptor);
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

/// A `ServerOffset` helper for tests/demo assertions.
pub fn max_server_offset(events: &[EventDescriptor<ItemEvent, ItemId>]) -> Option<ServerOffset> {
    events.iter().filter_map(|d| d.server_offset).max()
}
