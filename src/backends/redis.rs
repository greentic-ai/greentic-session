use crate::error::{SessionResult, invalid_argument, not_found, redis_error, serde_error};
use crate::store::SessionStore;
use greentic_types::{SessionData, SessionKey, TenantCtx, UserId};
use redis::{Client, Commands, Connection};
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

    fn user_lookup_key(&self, ctx: &TenantCtx, user: &UserId) -> String {
        let team = ctx
            .team_id
            .as_ref()
            .or(ctx.team.as_ref())
            .map(|v| v.as_str())
            .unwrap_or("-");
        format!(
            "{}:user:{}:{}:{}:{}",
            self.namespace,
            ctx.env.as_str(),
            ctx.tenant_id.as_str(),
            team,
            user.as_str()
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

    fn serialize(data: &SessionData) -> SessionResult<String> {
        serde_json::to_string(data).map_err(serde_error)
    }

    fn deserialize(payload: String) -> SessionResult<SessionData> {
        serde_json::from_str(&payload).map_err(serde_error)
    }

    fn mapping_sources<'a>(
        ctx_hint: Option<&'a TenantCtx>,
        data: &'a SessionData,
    ) -> Option<(&'a TenantCtx, UserId)> {
        if let Some(user) = data
            .tenant_ctx
            .user_id
            .clone()
            .or_else(|| data.tenant_ctx.user.clone())
        {
            Some((&data.tenant_ctx, user))
        } else {
            ctx_hint.and_then(|ctx| {
                ctx.user_id
                    .clone()
                    .or_else(|| ctx.user.clone())
                    .map(|user| (ctx, user))
            })
        }
    }

    fn store_user_mapping(
        &self,
        conn: &mut Connection,
        ctx_hint: Option<&TenantCtx>,
        data: &SessionData,
        key: &SessionKey,
    ) -> SessionResult<()> {
        if let Some((ctx, user)) = Self::mapping_sources(ctx_hint, data) {
            let lookup_key = self.user_lookup_key(ctx, &user);
            conn.set::<_, _, ()>(lookup_key, key.as_str())
                .map_err(redis_error)?;
        }
        Ok(())
    }

    fn remove_user_mapping(
        &self,
        conn: &mut Connection,
        data: &SessionData,
        key: &SessionKey,
    ) -> SessionResult<()> {
        if let Some((ctx, user)) = Self::mapping_sources(None, data) {
            let lookup_key = self.user_lookup_key(ctx, &user);
            let stored: Option<String> = conn.get(&lookup_key).map_err(redis_error)?;
            if stored
                .as_deref()
                .map(|value| value == key.as_str())
                .unwrap_or(false)
            {
                let _: () = conn.del(lookup_key).map_err(redis_error)?;
            }
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
        self.store_user_mapping(&mut conn, Some(ctx), &data, &key)?;
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
            .map_err(redis_error)?;
        self.remove_user_mapping(&mut conn, &previous, key)?;
        self.store_user_mapping(&mut conn, None, &data, key)
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
        self.remove_user_mapping(&mut conn, &data, key)
    }

    fn find_by_user(
        &self,
        ctx: &TenantCtx,
        user: &UserId,
    ) -> SessionResult<Option<(SessionKey, SessionData)>> {
        let mut conn = self.conn()?;
        let lookup_key = self.user_lookup_key(ctx, user);
        let stored: Option<String> = conn.get(&lookup_key).map_err(redis_error)?;
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
                        && stored_user != user
                    {
                        let _: () = conn.del(&lookup_key).map_err(redis_error)?;
                        return Ok(None);
                    }
                    Ok(Some((session_key, data)))
                } else {
                    let _: () = conn.del(&lookup_key).map_err(redis_error)?;
                    Ok(None)
                }
            }
            None => {
                let _: () = conn.del(&lookup_key).map_err(redis_error)?;
                Ok(None)
            }
        }
    }
}
