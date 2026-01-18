use crate::ReplyScope;
use crate::error::SessionResult;
use greentic_types::{SessionData, SessionKey, TenantCtx, UserId};
use std::time::Duration;

/// Persistent session storage interface used by Greentic runtimes.
///
/// `SessionData` captures the tenant context, flow identifier, cursor, and serialized execution
/// state snapshot for an in-flight flow. Implementations store that payload so runners can pause
/// execution, persist the snapshot, and resume the flow consistently after new input arrives.
pub trait SessionStore: Send + Sync + 'static {
    /// Creates a new session associated with the supplied tenant context and returns its key.
    fn create_session(&self, ctx: &TenantCtx, data: SessionData) -> SessionResult<SessionKey>;

    /// Fetches the session payload for the provided key, if it exists.
    fn get_session(&self, key: &SessionKey) -> SessionResult<Option<SessionData>>;

    /// Replaces the session payload for the provided key.
    fn update_session(&self, key: &SessionKey, data: SessionData) -> SessionResult<()>;

    /// Removes the session entry and clears any lookup indices.
    fn remove_session(&self, key: &SessionKey) -> SessionResult<()>;

    /// Registers a paused flow wait, persisting the session and routing indices.
    fn register_wait(
        &self,
        ctx: &TenantCtx,
        user_id: &UserId,
        scope: &ReplyScope,
        session_key: &SessionKey,
        data: SessionData,
        ttl: Option<Duration>,
    ) -> SessionResult<()>;

    /// Finds a wait registered for the provided scope, if one exists.
    fn find_wait_by_scope(
        &self,
        ctx: &TenantCtx,
        user_id: &UserId,
        scope: &ReplyScope,
    ) -> SessionResult<Option<SessionKey>>;

    /// Lists all waits registered for the provided user.
    fn list_waits_for_user(
        &self,
        ctx: &TenantCtx,
        user_id: &UserId,
    ) -> SessionResult<Vec<SessionKey>>;

    /// Clears a wait registration for the provided scope.
    fn clear_wait(
        &self,
        ctx: &TenantCtx,
        user_id: &UserId,
        scope: &ReplyScope,
    ) -> SessionResult<()>;

    /// Finds the active session bound to the specified tenant + user combination.
    #[deprecated(note = "use find_wait_by_scope or list_waits_for_user instead")]
    fn find_by_user(
        &self,
        ctx: &TenantCtx,
        user: &UserId,
    ) -> SessionResult<Option<(SessionKey, SessionData)>>;
}
