use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

/// Unique identifier for a session.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub Uuid);

impl SessionId {
    /// Generates a fresh session identifier.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Returns the raw UUID.
    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

/// Stable key used for routing (derived from connectors’ events).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionKey(pub String);

impl SessionKey {
    /// Borrows the underlying key as `&str`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Cursor tracks where to resume a flow.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionCursor {
    pub flow_id: String,
    pub node_id: String,
    pub wait_reason: Option<String>,
    pub outbox_seq: u64,
}

/// Outbox entry, deduped by (seq, payload hash).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxEntry {
    pub seq: u64,
    pub payload_sha256: [u8; 32],
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionMeta {
    pub tenant_id: String,
    pub team_id: Option<String>,
    pub user_id: Option<String>,
    pub labels: serde_json::Map<String, serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub key: SessionKey,
    pub cursor: SessionCursor,
    pub meta: SessionMeta,
    pub outbox: Vec<OutboxEntry>,
    pub updated_at: OffsetDateTime,
    pub ttl_secs: u32,
}

impl Session {
    /// Returns the tenant identifier for convenience.
    pub fn tenant_id(&self) -> &str {
        &self.meta.tenant_id
    }

    /// Applies in-place cleanup such as outbox deduplication and ttl normalization.
    pub fn normalize(&mut self) {
        self.dedupe_outbox();
        if self.ttl_secs == 0 {
            // A zero TTL is treated as "never expire".
            self.ttl_secs = 0;
        }
    }

    /// Deduplicates the outbox by `(seq, payload_sha256)` while maintaining first-wins ordering.
    pub fn dedupe_outbox(&mut self) {
        let mut seen = HashSet::new();
        self.outbox
            .retain(|entry| seen.insert((entry.seq, entry.payload_sha256)));
    }

    /// Returns the computed expiry deadline based on `updated_at` + `ttl_secs`.
    pub fn expires_at(&self) -> Option<OffsetDateTime> {
        if self.ttl_secs == 0 {
            return None;
        }
        let ttl = Duration::seconds(self.ttl_secs as i64);
        Some(self.updated_at + ttl)
    }
}

/// Compare-And-Set token; increments on each write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cas(pub u64);

impl Cas {
    /// Initial CAS value assigned to newly created records.
    pub const fn initial() -> Self {
        Self(1)
    }

    /// Sentinel CAS used when the value is absent.
    pub const fn none() -> Self {
        Self(0)
    }

    /// Returns the raw CAS counter.
    pub const fn value(self) -> u64 {
        self.0
    }

    /// Produces the next CAS value.
    pub const fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

impl From<u64> for Cas {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<Cas> for u64 {
    fn from(cas: Cas) -> Self {
        cas.0
    }
}
