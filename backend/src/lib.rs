//! Backend: the real event server. An in-memory `EventLog` (pure storage) exposed
//! over HTTP with axum. Swapping the in-memory log for Postgres wouldn't touch the
//! handlers — see [`shared::EventLog`].

pub mod event_log;
pub mod server;
