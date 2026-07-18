//! A deliberately boring demo domain: an inventory `Item`.
//!
//! This is the *only* domain-specific code in the repo. Notice how little there
//! is: an event enum, a single `apply_event`, and domain methods that record
//! events. Offline capture, history rebuild, and sync all come from the shared
//! core. Domain events carry only *when* the change happened — replica id and
//! write offset are provenance, stamped later by storage and the event log.

use serde::{Deserialize, Serialize};

use crate::core::{EntityEvent, EntityId, Event, EventSourcedEntity};

/// Everything that can happen to an `Item`, as facts in the past tense.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ItemEvent {
    Created { name: String, quantity: i64 },
    Renamed { name: String },
    QuantityChanged { quantity: i64 },
    NoteChanged { note: Option<String> },
    Deleted,
}
impl Event for ItemEvent {}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct ItemId(pub uuid::Uuid);
impl EntityId for ItemId {}

/// The current, folded state of an item. Derivative — always reconstructable
/// from its event history.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Item {
    pub id: ItemId,
    pub name: String,
    pub quantity: i64,
    pub note: Option<String>,
    /// Soft-delete. Deletion is terminal: once set, later edits are ignored, which
    /// is how "deletion wins over concurrent edits" falls out for free.
    pub deleted: bool,
    pub last_modified_ms: i64,
    uncommitted: Vec<EntityEvent<ItemEvent>>,
}

/// Domain operations. Each one records a domain event via `new_event`, which
/// applies it immediately (so in-memory state is current) and keeps it in
/// `uncommitted_events` until a storage persists it.
impl Item {
    pub fn create(id: ItemId, name: String, quantity: i64, now_ms: i64) -> Self {
        let mut item = Item {
            id,
            ..Default::default()
        };
        item.new_event(EntityEvent {
            event: ItemEvent::Created { name, quantity },
            modified_at_ms: now_ms,
        });
        item
    }

    pub fn set_quantity(&mut self, quantity: i64, now_ms: i64) {
        self.new_event(EntityEvent {
            event: ItemEvent::QuantityChanged { quantity },
            modified_at_ms: now_ms,
        });
    }

    pub fn rename(&mut self, name: String, now_ms: i64) {
        self.new_event(EntityEvent {
            event: ItemEvent::Renamed { name },
            modified_at_ms: now_ms,
        });
    }

    pub fn set_note(&mut self, note: Option<String>, now_ms: i64) {
        self.new_event(EntityEvent {
            event: ItemEvent::NoteChanged { note },
            modified_at_ms: now_ms,
        });
    }

    pub fn delete(&mut self, now_ms: i64) {
        self.new_event(EntityEvent {
            event: ItemEvent::Deleted,
            modified_at_ms: now_ms,
        });
    }
}

impl EventSourcedEntity for Item {
    type Evt = ItemEvent;
    type EntId = ItemId;

    fn uncommitted_events(&mut self) -> &mut Vec<EntityEvent<ItemEvent>> {
        &mut self.uncommitted
    }

    fn id(&self) -> &ItemId {
        &self.id
    }

    fn apply_event(&mut self, event: ItemEvent, modified_at_ms: i64) {
        // Deletion is terminal — this single guard is the whole "deletion wins"
        // rule. An edit ordered after a delete is dropped; an edit ordered before
        // is overwritten by the delete. Either way, delete wins.
        if self.deleted {
            return;
        }
        self.last_modified_ms = modified_at_ms;
        match event {
            ItemEvent::Created { name, quantity } => {
                self.name = name;
                self.quantity = quantity;
            }
            ItemEvent::Renamed { name } => self.name = name,
            ItemEvent::QuantityChanged { quantity } => self.quantity = quantity,
            ItemEvent::NoteChanged { note } => self.note = note,
            ItemEvent::Deleted => self.deleted = true,
        }
    }

    fn build_from_history(
        events: Vec<crate::core::EventDescriptor<ItemEvent, ItemId>>,
    ) -> Self {
        let mut entity = Self::default();
        if let Some(first) = events.first() {
            entity.id = first.entity_id.clone();
        }
        for event in events {
            let at = event.replica_time_ms;
            entity.apply_event(event.payload, at);
        }
        entity
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{sort_events, EventDescriptor};

    fn descriptor(
        entity: &ItemId,
        payload: ItemEvent,
        replica: &str,
        time_ms: i64,
        offset: u64,
    ) -> EventDescriptor<ItemEvent, ItemId> {
        EventDescriptor {
            replica_event_id: uuid::Uuid::new_v4(),
            replica_write_offset: offset,
            replica_time_ms: time_ms,
            replica_id: replica.to_string(),
            server_offset: None,
            server_time_ms: None,
            entity_id: entity.clone(),
            payload,
        }
    }

    #[test]
    fn folds_history_into_current_state() {
        let id = ItemId(uuid::Uuid::new_v4());
        let mut events = vec![
            descriptor(&id, ItemEvent::Created { name: "Wrench".into(), quantity: 10 }, "phone", 100, 0),
            descriptor(&id, ItemEvent::QuantityChanged { quantity: 8 }, "phone", 200, 0),
            descriptor(&id, ItemEvent::NoteChanged { note: Some("left bin".into()) }, "phone", 300, 0),
        ];
        sort_events(&mut events);
        let item = Item::build_from_history(events);

        assert_eq!(item.name, "Wrench");
        assert_eq!(item.quantity, 8);
        assert_eq!(item.note.as_deref(), Some("left bin"));
        assert!(!item.deleted);
    }

    #[test]
    fn two_replicas_converge_regardless_of_arrival_order() {
        let id = ItemId(uuid::Uuid::new_v4());
        let created = descriptor(&id, ItemEvent::Created { name: "Bolt".into(), quantity: 5 }, "server", 100, 0);
        // Two devices edit quantity while offline; phone_b's clock is later.
        let edit_a = descriptor(&id, ItemEvent::QuantityChanged { quantity: 7 }, "phone_a", 200, 0);
        let edit_b = descriptor(&id, ItemEvent::QuantityChanged { quantity: 9 }, "phone_b", 250, 0);

        // Replica 1 receives them in one order...
        let mut order1 = vec![created.clone(), edit_a.clone(), edit_b.clone()];
        // ...replica 2 in a shuffled order.
        let mut order2 = vec![edit_b, created, edit_a];

        sort_events(&mut order1);
        sort_events(&mut order2);
        let item1 = Item::build_from_history(order1);
        let item2 = Item::build_from_history(order2);

        // Both converge, and last-writer-by-clock (phone_b @250) wins.
        assert_eq!(item1, item2);
        assert_eq!(item1.quantity, 9);
    }

    #[test]
    fn deletion_wins_over_a_later_edit() {
        let id = ItemId(uuid::Uuid::new_v4());
        let mut events = vec![
            descriptor(&id, ItemEvent::Created { name: "Nut".into(), quantity: 3 }, "phone_a", 100, 0),
            descriptor(&id, ItemEvent::Deleted, "phone_a", 200, 0),
            // Another device edited it *after* (by clock) not knowing it was deleted.
            descriptor(&id, ItemEvent::QuantityChanged { quantity: 99 }, "phone_b", 300, 0),
        ];
        sort_events(&mut events);
        let item = Item::build_from_history(events);

        assert!(item.deleted);
        assert_eq!(item.quantity, 3); // the post-delete edit was dropped
    }
}
