//! `shared` — the offline-first sync core. Pure Rust domain logic with no platform
//! bindings: used directly by the `backend` (server) and by the `client-sdk`
//! (the mobile SDK, which adds the UniFFI layer for iOS/Android).
//!
//! The idea in one sentence: **don't store the item, store the events that
//! produced it.** Current state is a fold over an append-only event log, which
//! makes offline capture and multi-device sync fall out naturally.
//!
//! - [`core`] — the domain-agnostic event-sourcing machinery (`EventSourcedEntity`,
//!   the cross-replica ordering rule).
//! - [`event_log`] — the storage contract both the server and the device satisfy.
//! - [`item`] — a tiny demo domain (an inventory `Item`) implemented on top.
//!
//! The device-side sync engine is *not* here — it's client-only, so it lives in
//! the `client-sdk` crate.

pub mod core;
pub mod event_log;
pub mod item;
pub mod item_storage;
pub mod replica;

pub use core::{
    order_key, sort_events, EntityEvent, EntityId, Event, EventDescriptor, EventSourcedEntity,
    ServerOffset,
};
pub use event_log::{AppendEvent, EntityEventLog, EventsPage};
pub use item::{Item, ItemEvent, ItemId};
pub use item_storage::{ItemFilter, ItemStorage, EntityLogHandle, ItemStorageHandle};
pub use replica::ReplicaIdProvider;
