use crate::error::SessionResult;
use crate::error::{GreenticError, invalid_argument, not_found};
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

    fn normalize_team(ctx: &TenantCtx) -> Option<&TeamId> {
        ctx.team_id.as_ref().or(ctx.team.as_ref())
    }

    fn normalize_user(ctx: &TenantCtx) -> Option<&UserId> {
        ctx.user_id.as_ref().or(ctx.user.as_ref())
    }

    fn ctx_mismatch(expected: &TenantCtx, provided: &TenantCtx, reason: &str) -> GreenticError {
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
        let mut sessions = self.sessions.write();
        let Some(previous) = sessions.get(key).cloned() else {
            return Err(not_found(key));
        };
        Self::ensure_ctx_preserved(&previous.tenant_ctx, &data.tenant_ctx)?;
        sessions.insert(key.clone(), data.clone());
        drop(sessions);
        self.purge_user_mapping(&previous, key);
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
                let stored_ctx = &data.tenant_ctx;
                if stored_ctx.env == ctx.env
                    && stored_ctx.tenant_id == ctx.tenant_id
                    && Self::normalize_team(stored_ctx) == Self::normalize_team(ctx)
                {
                    if let Some(stored_user) = Self::normalize_user(stored_ctx)
                        && stored_user != user
                    {
                        self.user_index.write().remove(&lookup);
                        return Ok(None);
                    }
                    return Ok(Some((stored_key, data)));
                }
                self.user_index.write().remove(&lookup);
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
