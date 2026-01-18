use crate::ReplyScope;
use crate::error::{SessionResult, invalid_argument, not_found, redis_error, serde_error};
use crate::store::SessionStore;
use greentic_types::{SessionData, SessionKey, TenantCtx, UserId};
use redis::{Client, Commands, Connection};
use std::time::Duration;
use uuid::Uuid;

const DEFAULT_NAMESPACE: &str = "greentic:session";

/// Redis-backed session store that mirrors the in-memory semantics.
///
/// Constructors accept connection URLs or configuration strings only; no Redis
/// client types appear in the public API.
pub struct RedisSessionStore {
    client: Client,
    namespace: String,
}

impl RedisSessionStore {
    /// Creates a store using a Redis URL and the default namespace prefix.
    pub fn from_url(url: impl AsRef<str>) -> SessionResult<Self> {
        let client = Client::open(url.as_ref()).map_err(redis_error)?;
        Ok(Self::from_client_with_namespace(
            client,
            DEFAULT_NAMESPACE.to_string(),
        ))
    }

    /// Creates a store using a Redis URL and a custom namespace prefix.
    pub fn from_url_with_namespace(
        url: impl AsRef<str>,
        namespace: impl Into<String>,
    ) -> SessionResult<Self> {
        let client = Client::open(url.as_ref()).map_err(redis_error)?;
        Ok(Self::from_client_with_namespace(client, namespace.into()))
    }

    pub(crate) fn from_client_with_namespace(client: Client, namespace: impl Into<String>) -> Self {
        Self {
            client,
            namespace: namespace.into(),
        }
    }

    fn conn(&self) -> SessionResult<Connection> {
        self.client.get_connection().map_err(redis_error)
    }

    fn normalize_team(ctx: &TenantCtx) -> Option<&greentic_types::TeamId> {
        ctx.team_id.as_ref().or(ctx.team.as_ref())
    }

    fn normalize_user(ctx: &TenantCtx) -> Option<&UserId> {
        ctx.user_id.as_ref().or(ctx.user.as_ref())
    }

    fn ctx_mismatch(
        expected: &TenantCtx,
        provided: &TenantCtx,
        reason: &str,
    ) -> crate::error::GreenticError {
        let expected_team = Self::normalize_team(expected)
            .map(|t| t.as_str())
            .unwrap_or("-");
        let provided_team = Self::normalize_team(provided)
            .map(|t| t.as_str())
            .unwrap_or("-");
        let expected_user = Self::normalize_user(expected)
            .map(|u| u.as_str())
            .unwrap_or("-");
        let provided_user = Self::normalize_user(provided)
            .map(|u| u.as_str())
            .unwrap_or("-");
        invalid_argument(format!(
            "tenant context mismatch ({reason}): expected env={}, tenant={}, team={}, user={}, got env={}, tenant={}, team={}, user={}",
            expected.env.as_str(),
            expected.tenant_id.as_str(),
            expected_team,
            expected_user,
            provided.env.as_str(),
            provided.tenant_id.as_str(),
            provided_team,
            provided_user
        ))
    }

    fn session_entry_key(&self, key: &SessionKey) -> String {
        format!("{}:session:{}", self.namespace, key.as_str())
    }

    fn user_waits_key(&self, ctx: &TenantCtx, user: &UserId) -> String {
        let team = ctx
            .team_id
            .as_ref()
            .or(ctx.team.as_ref())
            .map(|v| v.as_str())
            .unwrap_or("-");
        format!(
            "{}:waits:user:{}:{}:{}:{}",
            self.namespace,
            ctx.env.as_str(),
            ctx.tenant_id.as_str(),
            team,
            user.as_str()
        )
    }

    fn scope_wait_key(&self, ctx: &TenantCtx, user: &UserId, scope: &ReplyScope) -> String {
        let team = ctx
            .team_id
            .as_ref()
            .or(ctx.team.as_ref())
            .map(|v| v.as_str())
            .unwrap_or("-");
        format!(
            "{}:waits:scope:{}:{}:{}:{}:{}",
            self.namespace,
            ctx.env.as_str(),
            ctx.tenant_id.as_str(),
            team,
            user.as_str(),
            scope.scope_hash()
        )
    }

    fn ensure_alignment(ctx: &TenantCtx, data: &SessionData) -> SessionResult<()> {
        let stored = &data.tenant_ctx;
        if ctx.env != stored.env || ctx.tenant_id != stored.tenant_id {
            return Err(Self::ctx_mismatch(stored, ctx, "env/tenant must match"));
        }
        if Self::normalize_team(ctx) != Self::normalize_team(stored) {
            return Err(Self::ctx_mismatch(stored, ctx, "team must match"));
        }
        if let Some(stored_user) = Self::normalize_user(stored) {
            let Some(provided_user) = Self::normalize_user(ctx) else {
                return Err(Self::ctx_mismatch(
                    stored,
                    ctx,
                    "user required by session but missing in caller context",
                ));
            };
            if stored_user != provided_user {
                return Err(Self::ctx_mismatch(
                    stored,
                    ctx,
                    "user must match stored session",
                ));
            }
        }
        Ok(())
    }

    fn ensure_ctx_preserved(existing: &TenantCtx, candidate: &TenantCtx) -> SessionResult<()> {
        if existing.env != candidate.env || existing.tenant_id != candidate.tenant_id {
            return Err(Self::ctx_mismatch(
                existing,
                candidate,
                "env/tenant cannot change for an existing session",
            ));
        }
        if Self::normalize_team(existing) != Self::normalize_team(candidate) {
            return Err(Self::ctx_mismatch(
                existing,
                candidate,
                "team cannot change for an existing session",
            ));
        }
        match (
            Self::normalize_user(existing),
            Self::normalize_user(candidate),
        ) {
            (Some(a), Some(b)) if a == b => {}
            (Some(_), Some(_)) | (Some(_), None) => {
                return Err(Self::ctx_mismatch(
                    existing,
                    candidate,
                    "user cannot change for an existing session",
                ));
            }
            (None, Some(_)) => {
                return Err(Self::ctx_mismatch(
                    existing,
                    candidate,
                    "user cannot be introduced when none was stored",
                ));
            }
            (None, None) => {}
        }
        Ok(())
    }

    fn ensure_user_matches(
        ctx: &TenantCtx,
        user: &UserId,
        data: &SessionData,
    ) -> SessionResult<()> {
        if let Some(ctx_user) = Self::normalize_user(ctx) {
            if ctx_user != user {
                return Err(invalid_argument(
                    "user must match tenant context when registering a wait",
                ));
            }
        }
        if let Some(stored_user) = Self::normalize_user(&data.tenant_ctx) {
            if stored_user != user {
                return Err(invalid_argument(
                    "user must match session data when registering a wait",
                ));
            }
        } else {
            return Err(invalid_argument(
                "user required by wait but missing in session data",
            ));
        }
        Ok(())
    }

    fn serialize(data: &SessionData) -> SessionResult<String> {
        serde_json::to_string(data).map_err(serde_error)
    }

    fn deserialize(payload: String) -> SessionResult<SessionData> {
        serde_json::from_str(&payload).map_err(serde_error)
    }

    fn apply_ttl(conn: &mut Connection, key: &str, ttl: Option<Duration>) -> SessionResult<()> {
        if let Some(ttl) = ttl {
            let ttl_ms = ttl.as_millis().max(1);
            let ttl_ms = if ttl_ms > i64::MAX as u128 {
                i64::MAX
            } else {
                ttl_ms as i64
            };
            conn.pexpire::<_, ()>(key, ttl_ms).map_err(redis_error)?;
        }
        Ok(())
    }
}

impl SessionStore for RedisSessionStore {
    fn create_session(&self, ctx: &TenantCtx, data: SessionData) -> SessionResult<SessionKey> {
        Self::ensure_alignment(ctx, &data)?;
        let key = SessionKey::new(Uuid::new_v4().to_string());
        let payload = Self::serialize(&data)?;
        let mut conn = self.conn()?;
        conn.set::<_, _, ()>(self.session_entry_key(&key), payload)
            .map_err(redis_error)?;
        Ok(key)
    }

    fn get_session(&self, key: &SessionKey) -> SessionResult<Option<SessionData>> {
        let mut conn = self.conn()?;
        let payload: Option<String> = conn.get(self.session_entry_key(key)).map_err(redis_error)?;
        payload.map(Self::deserialize).transpose()
    }

    fn update_session(&self, key: &SessionKey, data: SessionData) -> SessionResult<()> {
        let mut conn = self.conn()?;
        let entry_key = self.session_entry_key(key);
        let existing: Option<String> = conn.get(&entry_key).map_err(redis_error)?;
        let Some(existing_payload) = existing else {
            return Err(not_found(key));
        };
        let previous = Self::deserialize(existing_payload)?;
        Self::ensure_ctx_preserved(&previous.tenant_ctx, &data.tenant_ctx)?;
        let payload = Self::serialize(&data)?;
        conn.set::<_, _, ()>(&entry_key, payload)
            .map_err(redis_error)
    }

    fn remove_session(&self, key: &SessionKey) -> SessionResult<()> {
        let mut conn = self.conn()?;
        let entry_key = self.session_entry_key(key);
        let existing: Option<String> = conn.get(&entry_key).map_err(redis_error)?;
        let Some(payload) = existing else {
            return Err(not_found(key));
        };
        let data = Self::deserialize(payload)?;
        let _: () = conn.del(entry_key).map_err(redis_error)?;
        if let Some(user) = Self::normalize_user(&data.tenant_ctx) {
            let user_waits_key = self.user_waits_key(&data.tenant_ctx, user);
            let _: () = conn
                .srem::<_, _, ()>(user_waits_key, key.as_str())
                .map_err(redis_error)?;
        }
        Ok(())
    }

    fn register_wait(
        &self,
        ctx: &TenantCtx,
        user_id: &UserId,
        scope: &ReplyScope,
        session_key: &SessionKey,
        data: SessionData,
        ttl: Option<Duration>,
    ) -> SessionResult<()> {
        Self::ensure_alignment(ctx, &data)?;
        Self::ensure_user_matches(ctx, user_id, &data)?;
        let mut conn = self.conn()?;
        let entry_key = self.session_entry_key(session_key);
        let payload = Self::serialize(&data)?;
        conn.set::<_, _, ()>(&entry_key, payload)
            .map_err(redis_error)?;
        Self::apply_ttl(&mut conn, &entry_key, ttl)?;

        let user_waits_key = self.user_waits_key(ctx, user_id);
        conn.sadd::<_, _, ()>(&user_waits_key, session_key.as_str())
            .map_err(redis_error)?;

        let scope_key = self.scope_wait_key(ctx, user_id, scope);
        let previous: Option<String> = conn.get(&scope_key).map_err(redis_error)?;
        conn.set::<_, _, ()>(&scope_key, session_key.as_str())
            .map_err(redis_error)?;
        Self::apply_ttl(&mut conn, &scope_key, ttl)?;
        if let Some(previous) = previous {
            if previous != session_key.as_str() {
                let _: () = conn
                    .srem::<_, _, ()>(&user_waits_key, previous)
                    .map_err(redis_error)?;
            }
        }
        Ok(())
    }

    fn find_wait_by_scope(
        &self,
        ctx: &TenantCtx,
        user_id: &UserId,
        scope: &ReplyScope,
    ) -> SessionResult<Option<SessionKey>> {
        let mut conn = self.conn()?;
        let scope_key = self.scope_wait_key(ctx, user_id, scope);
        let stored: Option<String> = conn.get(&scope_key).map_err(redis_error)?;
        let Some(raw_key) = stored else {
            return Ok(None);
        };
        let session_key = SessionKey::new(raw_key);
        match self.get_session(&session_key)? {
            Some(data) => {
                let stored_ctx = &data.tenant_ctx;
                if stored_ctx.env == ctx.env
                    && stored_ctx.tenant_id == ctx.tenant_id
                    && Self::normalize_team(stored_ctx) == Self::normalize_team(ctx)
                {
                    if let Some(stored_user) = Self::normalize_user(stored_ctx)
                        && stored_user != user_id
                    {
                        let _: () = conn.del(&scope_key).map_err(redis_error)?;
                        let user_waits_key = self.user_waits_key(ctx, user_id);
                        let _: () = conn
                            .srem::<_, _, ()>(&user_waits_key, session_key.as_str())
                            .map_err(redis_error)?;
                        return Ok(None);
                    }
                    Ok(Some(session_key))
                } else {
                    let _: () = conn.del(&scope_key).map_err(redis_error)?;
                    let user_waits_key = self.user_waits_key(ctx, user_id);
                    let _: () = conn
                        .srem::<_, _, ()>(&user_waits_key, session_key.as_str())
                        .map_err(redis_error)?;
                    Ok(None)
                }
            }
            None => {
                let _: () = conn.del(&scope_key).map_err(redis_error)?;
                let user_waits_key = self.user_waits_key(ctx, user_id);
                let _: () = conn
                    .srem::<_, _, ()>(&user_waits_key, session_key.as_str())
                    .map_err(redis_error)?;
                Ok(None)
            }
        }
    }

    fn list_waits_for_user(
        &self,
        ctx: &TenantCtx,
        user_id: &UserId,
    ) -> SessionResult<Vec<SessionKey>> {
        let mut conn = self.conn()?;
        let user_waits_key = self.user_waits_key(ctx, user_id);
        let stored: Vec<String> = conn.smembers(&user_waits_key).map_err(redis_error)?;
        let mut results = Vec::new();
        for raw_key in stored {
            let session_key = SessionKey::new(raw_key.clone());
            match self.get_session(&session_key)? {
                Some(data) => {
                    let stored_ctx = &data.tenant_ctx;
                    if stored_ctx.env == ctx.env
                        && stored_ctx.tenant_id == ctx.tenant_id
                        && Self::normalize_team(stored_ctx) == Self::normalize_team(ctx)
                    {
                        if let Some(stored_user) = Self::normalize_user(stored_ctx)
                            && stored_user != user_id
                        {
                            let _: () = conn
                                .srem::<_, _, ()>(&user_waits_key, raw_key)
                                .map_err(redis_error)?;
                            continue;
                        }
                        results.push(session_key);
                    } else {
                        let _: () = conn
                            .srem::<_, _, ()>(&user_waits_key, raw_key)
                            .map_err(redis_error)?;
                    }
                }
                None => {
                    let _: () = conn
                        .srem::<_, _, ()>(&user_waits_key, raw_key)
                        .map_err(redis_error)?;
                }
            }
        }
        Ok(results)
    }

    fn clear_wait(
        &self,
        ctx: &TenantCtx,
        user_id: &UserId,
        scope: &ReplyScope,
    ) -> SessionResult<()> {
        let mut conn = self.conn()?;
        let scope_key = self.scope_wait_key(ctx, user_id, scope);
        let stored: Option<String> = conn.get(&scope_key).map_err(redis_error)?;
        if let Some(raw_key) = stored {
            let session_key = SessionKey::new(raw_key.clone());
            let entry_key = self.session_entry_key(&session_key);
            let _: () = conn.del(&entry_key).map_err(redis_error)?;
            let _: () = conn.del(&scope_key).map_err(redis_error)?;
            let user_waits_key = self.user_waits_key(ctx, user_id);
            let _: () = conn
                .srem::<_, _, ()>(&user_waits_key, raw_key)
                .map_err(redis_error)?;
        }
        Ok(())
    }

    fn find_by_user(
        &self,
        ctx: &TenantCtx,
        user: &UserId,
    ) -> SessionResult<Option<(SessionKey, SessionData)>> {
        let waits = self.list_waits_for_user(ctx, user)?;
        match waits.len() {
            0 => Ok(None),
            1 => {
                let key = waits.into_iter().next().expect("single wait entry");
                let data = self.get_session(&key)?.ok_or_else(|| not_found(&key))?;
                Ok(Some((key, data)))
            }
            _ => Err(invalid_argument(
                "multiple waits exist for user; use scope-based routing instead",
            )),
        }
    }
}
