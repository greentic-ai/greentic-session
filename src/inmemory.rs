use crate::error::SessionResult;
use crate::error::{invalid_argument, not_found};
use crate::store::SessionStore;
use greentic_types::{EnvId, SessionData, SessionKey, TeamId, TenantCtx, TenantId, UserId};
use parking_lot::RwLock;
use std::collections::HashMap;
use uuid::Uuid;

/// Simple in-memory implementation backed by hash maps.
pub struct InMemorySessionStore {
    sessions: RwLock<HashMap<SessionKey, SessionData>>,
    user_index: RwLock<HashMap<UserLookupKey, SessionKey>>,
}

impl Default for InMemorySessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemorySessionStore {
    /// Constructs an empty store.
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            user_index: RwLock::new(HashMap::new()),
        }
    }

    fn next_key() -> SessionKey {
        SessionKey::new(Uuid::new_v4().to_string())
    }

    fn ensure_alignment(ctx: &TenantCtx, data: &SessionData) -> SessionResult<()> {
        if ctx.env != data.tenant_ctx.env || ctx.tenant_id != data.tenant_ctx.tenant_id {
            return Err(invalid_argument(
                "session data tenant context does not match provided TenantCtx",
            ));
        }
        Ok(())
    }

    fn lookup_from_ctx(ctx: &TenantCtx) -> Option<UserLookupKey> {
        let user = ctx.user_id.clone().or_else(|| ctx.user.clone())?;
        Some(UserLookupKey::from_ctx(ctx, &user))
    }

    fn lookup_from_data(data: &SessionData) -> Option<UserLookupKey> {
        let user = data
            .tenant_ctx
            .user_id
            .clone()
            .or_else(|| data.tenant_ctx.user.clone())?;
        Some(UserLookupKey::from_ctx(&data.tenant_ctx, &user))
    }

    fn record_user_mapping(
        &self,
        ctx_hint: Option<&TenantCtx>,
        data: &SessionData,
        key: &SessionKey,
    ) {
        let lookup =
            Self::lookup_from_data(data).or_else(|| ctx_hint.and_then(Self::lookup_from_ctx));
        if let Some(entry) = lookup {
            self.user_index.write().insert(entry, key.clone());
        }
    }

    fn purge_user_mapping(&self, data: &SessionData, key: &SessionKey) {
        if let Some(entry) = Self::lookup_from_data(data) {
            let mut guard = self.user_index.write();
            if guard
                .get(&entry)
                .map(|existing| existing == key)
                .unwrap_or(false)
            {
                guard.remove(&entry);
            }
        }
    }
}

impl SessionStore for InMemorySessionStore {
    fn create_session(&self, ctx: &TenantCtx, data: SessionData) -> SessionResult<SessionKey> {
        Self::ensure_alignment(ctx, &data)?;
        let key = Self::next_key();
        self.sessions.write().insert(key.clone(), data.clone());
        self.record_user_mapping(Some(ctx), &data, &key);
        Ok(key)
    }

    fn get_session(&self, key: &SessionKey) -> SessionResult<Option<SessionData>> {
        Ok(self.sessions.read().get(key).cloned())
    }

    fn update_session(&self, key: &SessionKey, data: SessionData) -> SessionResult<()> {
        let previous = self.sessions.write().insert(key.clone(), data.clone());
        let Some(old) = previous else {
            return Err(not_found(key));
        };
        self.purge_user_mapping(&old, key);
        self.record_user_mapping(None, &data, key);
        Ok(())
    }

    fn remove_session(&self, key: &SessionKey) -> SessionResult<()> {
        if let Some(old) = self.sessions.write().remove(key) {
            self.purge_user_mapping(&old, key);
            Ok(())
        } else {
            Err(not_found(key))
        }
    }

    fn find_by_user(
        &self,
        ctx: &TenantCtx,
        user: &UserId,
    ) -> SessionResult<Option<(SessionKey, SessionData)>> {
        let lookup = UserLookupKey::from_ctx(ctx, user);
        if let Some(stored_key) = self.user_index.read().get(&lookup).cloned() {
            if let Some(data) = self.sessions.read().get(&stored_key).cloned() {
                return Ok(Some((stored_key, data)));
            }
            self.user_index.write().remove(&lookup);
        }
        Ok(None)
    }
}

#[derive(Clone, Eq, PartialEq, Hash)]
struct UserLookupKey {
    env: EnvId,
    tenant: TenantId,
    team: Option<TeamId>,
    user: UserId,
}

impl UserLookupKey {
    fn from_ctx(ctx: &TenantCtx, user: &UserId) -> Self {
        Self {
            env: ctx.env.clone(),
            tenant: ctx.tenant_id.clone(),
            team: ctx.team_id.clone().or_else(|| ctx.team.clone()),
            user: user.clone(),
        }
    }
}
