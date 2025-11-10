use crate::error::{invalid_argument, not_found, redis_error, serde_error};
use crate::store::SessionStore;
use greentic_types::{GResult, SessionData, SessionKey, TenantCtx, UserId};
use redis::{Commands, Connection};
use uuid::Uuid;

const DEFAULT_NAMESPACE: &str = "greentic:session";

/// Redis-backed session store that mirrors the in-memory semantics.
pub struct RedisSessionStore {
    client: redis::Client,
    namespace: String,
}

impl RedisSessionStore {
    /// Creates a store using the default namespace prefix.
    pub fn new(client: redis::Client) -> Self {
        Self::with_namespace(client, DEFAULT_NAMESPACE)
    }

    /// Creates a store with a custom namespace prefix.
    pub fn with_namespace(client: redis::Client, namespace: impl Into<String>) -> Self {
        Self {
            client,
            namespace: namespace.into(),
        }
    }

    fn conn(&self) -> GResult<Connection> {
        self.client.get_connection().map_err(redis_error)
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

    fn ensure_alignment(ctx: &TenantCtx, data: &SessionData) -> GResult<()> {
        if ctx.env != data.tenant_ctx.env || ctx.tenant_id != data.tenant_ctx.tenant_id {
            return Err(invalid_argument(
                "session data tenant context does not match provided TenantCtx",
            ));
        }
        Ok(())
    }

    fn serialize(data: &SessionData) -> GResult<String> {
        serde_json::to_string(data).map_err(serde_error)
    }

    fn deserialize(payload: String) -> GResult<SessionData> {
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
    ) -> GResult<()> {
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
    ) -> GResult<()> {
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
    fn create_session(&self, ctx: &TenantCtx, data: SessionData) -> GResult<SessionKey> {
        Self::ensure_alignment(ctx, &data)?;
        let key = SessionKey::new(Uuid::new_v4().to_string());
        let payload = Self::serialize(&data)?;
        let mut conn = self.conn()?;
        conn.set::<_, _, ()>(self.session_entry_key(&key), payload)
            .map_err(redis_error)?;
        self.store_user_mapping(&mut conn, Some(ctx), &data, &key)?;
        Ok(key)
    }

    fn get_session(&self, key: &SessionKey) -> GResult<Option<SessionData>> {
        let mut conn = self.conn()?;
        let payload: Option<String> = conn.get(self.session_entry_key(key)).map_err(redis_error)?;
        payload.map(Self::deserialize).transpose()
    }

    fn update_session(&self, key: &SessionKey, data: SessionData) -> GResult<()> {
        let mut conn = self.conn()?;
        let entry_key = self.session_entry_key(key);
        let existing: Option<String> = conn.get(&entry_key).map_err(redis_error)?;
        let Some(existing_payload) = existing else {
            return Err(not_found(key));
        };
        let previous = Self::deserialize(existing_payload)?;
        let payload = Self::serialize(&data)?;
        conn.set::<_, _, ()>(&entry_key, payload)
            .map_err(redis_error)?;
        self.remove_user_mapping(&mut conn, &previous, key)?;
        self.store_user_mapping(&mut conn, None, &data, key)
    }

    fn remove_session(&self, key: &SessionKey) -> GResult<()> {
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
    ) -> GResult<Option<(SessionKey, SessionData)>> {
        let mut conn = self.conn()?;
        let lookup_key = self.user_lookup_key(ctx, user);
        let stored: Option<String> = conn.get(&lookup_key).map_err(redis_error)?;
        let Some(raw_key) = stored else {
            return Ok(None);
        };
        let session_key = SessionKey::new(raw_key);
        match self.get_session(&session_key)? {
            Some(data) => Ok(Some((session_key, data))),
            None => {
                let _: () = conn.del(&lookup_key).map_err(redis_error)?;
                Ok(None)
            }
        }
    }
}
