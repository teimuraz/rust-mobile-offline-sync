//! The device-side sync engine.
//! This crate compiles for both
//! iOS and Android, so this is still "shared Rust" across the two mobile platforms.
//!
//! To sync, the device **pushes** the events it authored that the server hasn't
//! acknowledged yet (`server_offset IS NULL` in the local log), records the
//! server's acknowledgement (`mark_synced` stamps the assigned `server_offset`),
//! then **pulls** events after the last server offset it has seen. `append` is
//! idempotent, so a connection dropped mid-sync just means retry.

use shared::{AppendEvent, EntityId, Event, EventDescriptor, EventsPage, ItemEvent, ItemId, ServerOffset};

use crate::event_log::DeviceEventLog;

/// The error surfaced to Swift/Kotlin when a sync fails (e.g. the backend is
/// unreachable). UniFFI turns this into a thrown error — no panic, no crash.
#[derive(Debug, uniffi::Error)]
pub enum SyncError {
    Transport { message: String },
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncError::Transport { message } => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for SyncError {}

/// The two operations a device performs against the server, over whatever transport
/// (HTTP in the real app).
#[async_trait::async_trait]
pub trait ServerApi<E: Event, EntId: EntityId>: Send + Sync {
    /// Upload events to the server. Idempotent on the server side. Returns the
    /// events as stored — with the server-assigned `server_offset`, which the
    /// device records via `mark_synced`.
    async fn push(
        &self,
        events: Vec<AppendEvent<E, EntId>>,
    ) -> Result<Vec<EventDescriptor<E, EntId>>, SyncError>;

    /// Download events the server has stored after `offset` (oldest first, capped
    /// at `limit`), plus the cursor to continue from.
    async fn pull(
        &self,
        offset: Option<ServerOffset>,
        limit: usize,
    ) -> Result<EventsPage<E, EntId>, SyncError>;
}

/// Per-device sync state.
///
/// Only a **pull** cursor is needed: the push side has no cursor at all, because
/// "what still needs pushing" is marked on the events themselves — a local event
/// with `server_offset IS NULL` hasn't been acknowledged yet.
///
/// In this demo it lives in memory, so a restart re-syncs from scratch (harmless —
/// `append` is idempotent). In a real app it's persisted in SQLite alongside the
/// event log, so sync resumes exactly where it left off across app launches.
pub struct SyncState {
    replica_id: String,
    /// Highest `server_offset` pulled so far — "I've seen the server's stream up
    /// to here".
    pulled: Option<ServerOffset>,
}

impl SyncState {
    pub fn new(replica_id: impl Into<String>) -> Self {
        Self {
            replica_id: replica_id.into(),
            pulled: None,
        }
    }
}

/// One full sync round for a device: push local changes, then pull remote ones.
///
/// Returns the entity ids changed by the pull, so the caller can refresh just those
/// in its read model instead of rebuilding everything.
pub async fn sync(
    local: &mut DeviceEventLog,
    server: &dyn ServerApi<ItemEvent, ItemId>,
    state: &mut SyncState,
    batch: usize,
) -> Result<Vec<ItemId>, SyncError> {
    push(local, server, state, batch).await?;
    pull(local, server, state, batch).await
}

/// Upload local events the server hasn't acknowledged (`server_offset IS NULL`),
/// then record the acknowledgement so they won't be pushed again.
async fn push(
    local: &mut DeviceEventLog,
    server: &dyn ServerApi<ItemEvent, ItemId>,
    state: &mut SyncState,
    batch: usize,
) -> Result<(), SyncError> {
    loop {
        let unsynced = local.get_unsynced(&state.replica_id, batch).await;
        if unsynced.is_empty() {
            break;
        }
        let events: Vec<AppendEvent<ItemEvent, ItemId>> =
            unsynced.iter().map(AppendEvent::from).collect();
        // On failure we return WITHOUT marking anything synced, so these events
        // are retried on the next sync — nothing is lost.
        let acknowledged = server.push(events).await?;
        local.mark_synced(&acknowledged).await;
    }
    Ok(())
}

/// Download everything the server has after our `pulled` cursor and fold it into
/// the local log. `append` dedupes by event id, so re-pulling is harmless. Returns
/// the entity ids touched by the incoming events.
async fn pull(
    local: &mut DeviceEventLog,
    server: &dyn ServerApi<ItemEvent, ItemId>,
    state: &mut SyncState,
    batch: usize,
) -> Result<Vec<ItemId>, SyncError> {
    let mut touched = Vec::new();
    loop {
        let page = server.pull(state.pulled, batch).await?;
        if page.events.is_empty() {
            break;
        }
        for d in &page.events {
            touched.push(d.entity_id.clone());
        }
        let incoming: Vec<AppendEvent<ItemEvent, ItemId>> =
            page.events.iter().map(AppendEvent::from).collect();
        local.append(incoming).await;
        state.pulled = page.next_offset;
    }
    Ok(touched)
}
