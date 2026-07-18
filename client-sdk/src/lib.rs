//! `client-sdk` — the mobile SDK
//!
//! ```text
//! ffi (ItemService, uniffi)          ← what Swift/Kotlin see; no logic
//!   └─ item_service (SdkItemService) ← local business logic, fully offline
//!        └─ shared::ItemStorage         ← store → event log + projection table;
//!                                         reads → projection table only
//! sync_runner (SyncRunner, uniffi)   ← background job (or manual sync_now);
//!   └─ sync + http                      works on the event log, rebuilds
//!                                       projections via the storage, notifies the
//!                                       app via SyncListener when data changed
//! stores (Stores, uniffi)            ← wires log + storage (like the real
//!                                       SdkEventSourcedStores)
//! ```
//!
//! The domain (`Item`, its events, the event-sourcing core, `EventLog`,
//! `ItemStorage`) lives in `shared`, which the backend uses too. Everything in
//! this crate is client-only Rust, compiled for both iOS and Android.

uniffi::setup_scaffolding!();

mod event_log;
mod ffi;
mod http;
mod item_service;
mod sync;
mod sync_runner;

pub use event_log::{DeviceEventLog, DeviceLogHandle};
pub use ffi::{EventSourcedStores, ItemService, ItemView};
pub use item_service::SdkItemService;
pub use sync::SyncError;
pub use sync_runner::{SyncListener, SyncRunner};
