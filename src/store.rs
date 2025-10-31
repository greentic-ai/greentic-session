use crate::model::{Cas, Session, SessionKey};
use greentic_types::GResult;

#[cfg(feature = "schema")]
use schemars::JsonSchema;

#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub trait SessionStore: Send + Sync + 'static {
    /// Fetch by key; returns `(Session, Cas)` if present.
    fn get(&self, key: &SessionKey) -> GResult<Option<(Session, Cas)>>;

    /// Create or replace, returning the new `Cas`.
    fn put(&self, session: Session) -> GResult<Cas>;

    /// Update using CAS—only writes if `expected` matches the stored Cas.
    fn update_cas(&self, session: Session, expected: Cas) -> GResult<Result<Cas, Cas>>;

    /// Delete by key; return true if something was deleted.
    fn delete(&self, key: &SessionKey) -> GResult<bool>;

    /// Refresh TTL or `updated_at` without modifying payload.
    fn touch(&self, key: &SessionKey, ttl_secs: Option<u32>) -> GResult<bool>;
}
