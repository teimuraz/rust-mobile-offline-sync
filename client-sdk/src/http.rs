//! `HttpServerApi` — the device's HTTP client to the server.
//!
//! This is shared Rust: it runs unchanged on iOS and Android (the FFI layer just
//! exposes the components that use it). It implements the `ServerApi` trait, so
//! the sync engine drives it like any other transport — `push` is an HTTP POST,
//! `pull` an HTTP GET, against the backend's `/events` endpoint.

use shared::{AppendEvent, EventDescriptor, EventsPage, ItemEvent, ItemId, ServerOffset};

use crate::sync::{ServerApi, SyncError};

pub struct HttpServerApi {
    base_url: String,
    http: reqwest::Client,
}

impl HttpServerApi {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            http: reqwest::Client::new(),
        }
    }

    fn events_url(&self) -> String {
        format!("{}/events", self.base_url)
    }
}

#[async_trait::async_trait]
impl ServerApi<ItemEvent, ItemId> for HttpServerApi {
    async fn push(
        &self,
        events: Vec<AppendEvent<ItemEvent, ItemId>>,
    ) -> Result<Vec<EventDescriptor<ItemEvent, ItemId>>, SyncError> {
        if events.is_empty() {
            return Ok(Vec::new());
        }
        self.http
            .post(self.events_url())
            .json(&events)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(to_error)?
            .json()
            .await
            .map_err(to_error)
    }

    async fn pull(
        &self,
        offset: Option<ServerOffset>,
        limit: usize,
    ) -> Result<EventsPage<ItemEvent, ItemId>, SyncError> {
        let mut url = format!("{}?limit={}", self.events_url(), limit);
        if let Some(o) = offset {
            url.push_str(&format!("&after={}", o.0));
        }
        self.http
            .get(url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(to_error)?
            .json()
            .await
            .map_err(to_error)
    }
}

fn to_error(e: reqwest::Error) -> SyncError {
    SyncError::Transport {
        message: e.to_string(),
    }
}
