//! Replica identity — who authored an event. Mirrors the real SDK's
//! `ReplicaIdProvider`: the storage asks it for the id at store time; domain code
//! never sees it. (In the real app the id is generated once per install and kept
//! in the database; here it's just configured.)

#[derive(Clone)]
pub struct ReplicaIdProvider {
    replica_id: String,
}

impl ReplicaIdProvider {
    pub fn new(replica_id: impl Into<String>) -> Self {
        Self {
            replica_id: replica_id.into(),
        }
    }

    pub fn get(&self) -> String {
        self.replica_id.clone()
    }
}
