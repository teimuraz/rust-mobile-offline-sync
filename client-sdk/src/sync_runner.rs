//! Runs either on a fixed interval (background tokio task) or on demand
//! (`sync_now`). When a sync pulls changes, the app is notified through the
//! `SyncListener` callback (implemented in Swift/Kotlin) so the UI can refresh.
//!
//! The runner also tracks **network status** (the demo-scale analog of the real
//! SDK's network-status watcher): while offline, sync ticks are skipped; the
//! moment the app comes back online, a sync round fires immediately — that's the
//! offline-first "sync when connectivity returns" behavior.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use shared::ItemStorageHandle;
use tokio::sync::Mutex;

use crate::event_log::DeviceLogHandle;
use crate::ffi::EventSourcedStores;
use crate::http::HttpServerApi;
use crate::sync::{sync, SyncError, SyncState};

/// Implemented by the app (Swift/Kotlin). Called from the runner's background
/// task whenever a sync changed local data.
#[uniffi::export(with_foreign)]
pub trait SyncListener: Send + Sync {
    fn on_items_changed(&self);
}

#[derive(uniffi::Object)]
pub struct SyncRunner {
    item_log: DeviceLogHandle,
    item_storage: ItemStorageHandle,
    sync_state: Arc<Mutex<SyncState>>,
    server: Arc<HttpServerApi>,
    running: Arc<AtomicBool>,
    /// Simulated network status. In the real app this is fed by the platform's
    /// reachability watcher instead of a UI toggle.
    online: Arc<AtomicBool>,
    listener: Arc<StdMutex<Option<Arc<dyn SyncListener>>>>,
}

#[uniffi::export(async_runtime = "tokio")]
impl SyncRunner {
    /// `server_url` is the backend base URL, e.g. "http://127.0.0.1:4000".
    #[uniffi::constructor]
    pub fn new(stores: Arc<EventSourcedStores>, server_url: String) -> Arc<Self> {
        Arc::new(Self {
            item_log: stores.item_log.clone(),
            item_storage: stores.item_storage.clone(),
            sync_state: Arc::new(Mutex::new(SyncState::new(stores.replica_id.clone()))),
            server: Arc::new(HttpServerApi::new(server_url)),
            running: Arc::new(AtomicBool::new(false)),
            online: Arc::new(AtomicBool::new(true)),
            listener: Arc::new(StdMutex::new(None)),
        })
    }

    /// Register the app-side callback fired when a sync changes local data.
    pub fn set_listener(&self, listener: Arc<dyn SyncListener>) {
        *self.listener.lock().unwrap() = Some(listener);
    }

    pub fn is_online(&self) -> bool {
        self.online.load(Ordering::SeqCst)
    }

    /// Simulate going online/offline. While offline, background ticks skip and
    /// `sync_now` fails fast — edits keep accumulating locally. Coming back
    /// online fires a sync round immediately (sync-on-reconnect).
    pub async fn set_online(&self, online: bool) {
        let was_online = self.online.swap(online, Ordering::SeqCst);
        if online && !was_online {
            // Reconnected: sync right away instead of waiting for the next tick.
            let item_log = self.item_log.clone();
            let item_storage = self.item_storage.clone();
            let sync_state = self.sync_state.clone();
            let server = self.server.clone();
            let listener = self.listener.clone();
            tokio::spawn(async move {
                if let Ok(true) = sync_once(&item_log, &item_storage, &server, &sync_state).await
                {
                    let listener = listener.lock().unwrap().clone();
                    if let Some(l) = listener {
                        l.on_items_changed();
                    }
                }
            });
        }
    }

    /// Start the background job: one sync round every `interval_ms`. Skipped
    /// while offline; failures (backend unreachable, …) are ignored — the next
    /// tick simply retries.
    pub async fn start(&self, interval_ms: u64) {
        if self.running.swap(true, Ordering::SeqCst) {
            return; // already running
        }
        let item_log = self.item_log.clone();
        let item_storage = self.item_storage.clone();
        let sync_state = self.sync_state.clone();
        let server = self.server.clone();
        let running = self.running.clone();
        let online = self.online.clone();
        let listener = self.listener.clone();

        tokio::spawn(async move {
            while running.load(Ordering::SeqCst) {
                if online.load(Ordering::SeqCst) {
                    match sync_once(&item_log, &item_storage, &server, &sync_state).await {
                        Ok(changed) => {
                            if changed {
                                let listener = listener.lock().unwrap().clone();
                                if let Some(l) = listener {
                                    l.on_items_changed();
                                }
                            }
                        }
                        Err(_) => { /* backend down — retry next tick */ }
                    }
                }
                tokio::time::sleep(Duration::from_millis(interval_ms)).await;
            }
        });
    }

    /// Stop the background job (the current tick finishes, then the task exits).
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// One manual sync round. Throws if offline or the backend is unreachable;
    /// local state is untouched and a later sync retries.
    pub async fn sync_now(&self) -> Result<(), SyncError> {
        if !self.is_online() {
            return Err(SyncError::Transport {
                message: "offline (simulated)".to_string(),
            });
        }
        let changed = sync_once(
            &self.item_log,
            &self.item_storage,
            &self.server,
            &self.sync_state,
        )
        .await?;
        if changed {
            let listener = self.listener.lock().unwrap().clone();
            if let Some(l) = listener {
                l.on_items_changed();
            }
        }
        Ok(())
    }
}

/// One full round: push local events, pull remote ones (all against the event
/// log), then have the storage rebuild the projections the pull touched. Returns
/// whether anything changed locally.
///
/// Note: the log lock is held across the HTTP round-trip — fine for a demo; a
/// production runner stages events outside the lock.
async fn sync_once(
    item_log: &DeviceLogHandle,
    item_storage: &ItemStorageHandle,
    server: &HttpServerApi,
    sync_state: &Mutex<SyncState>,
) -> Result<bool, SyncError> {
    let touched = {
        let mut log = item_log.lock().await;
        let mut state = sync_state.lock().await;
        sync(&mut *log, server, &mut state, 500).await?
    };
    if !touched.is_empty() {
        let mut storage = item_storage.lock().await;
        for id in &touched {
            storage.build_projection(id).await;
        }
    }
    Ok(!touched.is_empty())
}
