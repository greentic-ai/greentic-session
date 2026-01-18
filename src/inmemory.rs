use crate::ReplyScope;
use crate::error::SessionResult;
use crate::error::{GreenticError, invalid_argument, not_found};
use crate::store::SessionStore;
use greentic_types::{EnvId, SessionData, SessionKey, TeamId, TenantCtx, TenantId, UserId};
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Simple in-memory implementation backed by hash maps.
pub struct InMemorySessionStore {
    sessions: RwLock<HashMap<SessionKey, SessionEntry>>,
    user_waits: RwLock<HashMap<UserLookupKey, HashSet<SessionKey>>>,
    scope_index: RwLock<HashMap<ScopeLookupKey, ScopeEntry>>,
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
            user_waits: RwLock::new(HashMap::new()),
            scope_index: RwLock::new(HashMap::new()),
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

    fn ttl_deadline(ttl: Option<Duration>) -> Option<Instant> {
        ttl.map(|value| Instant::now() + value)
    }

    fn is_expired(deadline: Option<Instant>) -> bool {
        deadline
            .map(|value| Instant::now() >= value)
            .unwrap_or(false)
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

    fn ensure_user_matches(
        ctx: &TenantCtx,
        user: &UserId,
        data: &SessionData,
    ) -> SessionResult<()> {
        if let Some(ctx_user) = Self::normalize_user(ctx)
            && ctx_user != user
        {
            return Err(invalid_argument(
                "user must match tenant context when registering a wait",
            ));
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

    fn user_lookup_key(ctx: &TenantCtx, user: &UserId) -> UserLookupKey {
        UserLookupKey::from_ctx(ctx, user)
    }

    fn scope_lookup_key(ctx: &TenantCtx, user: &UserId, scope: &ReplyScope) -> ScopeLookupKey {
        ScopeLookupKey::from_ctx(ctx, user, scope)
    }

    fn remove_from_user_waits(&self, lookup: &UserLookupKey, key: &SessionKey) {
        let mut waits = self.user_waits.write();
        if let Some(entries) = waits.get_mut(lookup) {
            entries.remove(key);
            if entries.is_empty() {
                waits.remove(lookup);
            }
        }
    }

    fn remove_scope_entry(&self, scope_key: &ScopeLookupKey) -> Option<ScopeEntry> {
        self.scope_index.write().remove(scope_key)
    }

    fn purge_expired_session(&self, key: &SessionKey, entry: SessionEntry) {
        if let Some(user_lookup) = &entry.wait_user {
            self.remove_from_user_waits(user_lookup, key);
        }
        if let Some(scope_key) = &entry.scope_key {
            self.remove_scope_entry(scope_key);
        }
    }
}

impl SessionStore for InMemorySessionStore {
    fn create_session(&self, ctx: &TenantCtx, data: SessionData) -> SessionResult<SessionKey> {
        Self::ensure_alignment(ctx, &data)?;
        let key = Self::next_key();
        let entry = SessionEntry {
            data: data.clone(),
            expires_at: None,
            wait_user: None,
            scope_key: None,
        };
        self.sessions.write().insert(key.clone(), entry);
        Ok(key)
    }

    fn get_session(&self, key: &SessionKey) -> SessionResult<Option<SessionData>> {
        let mut sessions = self.sessions.write();
        let Some(entry) = sessions.get(key).cloned() else {
            return Ok(None);
        };
        if Self::is_expired(entry.expires_at) {
            sessions.remove(key);
            drop(sessions);
            self.purge_expired_session(key, entry);
            return Ok(None);
        }
        Ok(Some(entry.data))
    }

    fn update_session(&self, key: &SessionKey, data: SessionData) -> SessionResult<()> {
        let mut sessions = self.sessions.write();
        let Some(previous) = sessions.get(key).cloned() else {
            return Err(not_found(key));
        };
        Self::ensure_ctx_preserved(&previous.data.tenant_ctx, &data.tenant_ctx)?;
        let entry = SessionEntry {
            data: data.clone(),
            expires_at: previous.expires_at,
            wait_user: previous.wait_user.clone(),
            scope_key: previous.scope_key.clone(),
        };
        sessions.insert(key.clone(), entry);
        Ok(())
    }

    fn remove_session(&self, key: &SessionKey) -> SessionResult<()> {
        if let Some(old) = self.sessions.write().remove(key) {
            self.purge_expired_session(key, old);
            Ok(())
        } else {
            Err(not_found(key))
        }
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
        let user_lookup = Self::user_lookup_key(ctx, user_id);
        let scope_key = Self::scope_lookup_key(ctx, user_id, scope);
        let expires_at = Self::ttl_deadline(ttl);

        let existing = self.sessions.read().get(session_key).cloned();
        if let Some(existing) = &existing {
            Self::ensure_ctx_preserved(&existing.data.tenant_ctx, &data.tenant_ctx)?;
            if let Some(existing_user) = &existing.wait_user {
                self.remove_from_user_waits(existing_user, session_key);
            }
            if let Some(existing_scope) = &existing.scope_key {
                self.remove_scope_entry(existing_scope);
            }
        }
        let entry = SessionEntry {
            data,
            expires_at,
            wait_user: Some(user_lookup.clone()),
            scope_key: Some(scope_key.clone()),
        };
        self.sessions.write().insert(session_key.clone(), entry);

        let mut waits = self.user_waits.write();
        waits
            .entry(user_lookup.clone())
            .or_default()
            .insert(session_key.clone());
        drop(waits);

        let mut scopes = self.scope_index.write();
        if let Some(existing) = scopes.get(&scope_key)
            && existing.session_key != *session_key
        {
            self.remove_from_user_waits(&user_lookup, &existing.session_key);
        }
        scopes.insert(
            scope_key,
            ScopeEntry {
                session_key: session_key.clone(),
                expires_at,
            },
        );
        Ok(())
    }

    fn find_wait_by_scope(
        &self,
        ctx: &TenantCtx,
        user_id: &UserId,
        scope: &ReplyScope,
    ) -> SessionResult<Option<SessionKey>> {
        let scope_key = Self::scope_lookup_key(ctx, user_id, scope);
        let entry = self.scope_index.read().get(&scope_key).cloned();
        let Some(entry) = entry else {
            return Ok(None);
        };
        if Self::is_expired(entry.expires_at) {
            self.remove_scope_entry(&scope_key);
            self.remove_from_user_waits(&UserLookupKey::from_ctx(ctx, user_id), &entry.session_key);
            let removed = self.sessions.write().remove(&entry.session_key);
            if let Some(session_entry) = removed {
                self.purge_expired_session(&entry.session_key, session_entry);
            }
            return Ok(None);
        }
        let Some(session) = self.get_session(&entry.session_key)? else {
            self.remove_scope_entry(&scope_key);
            self.remove_from_user_waits(&UserLookupKey::from_ctx(ctx, user_id), &entry.session_key);
            return Ok(None);
        };
        let stored_ctx = &session.tenant_ctx;
        if stored_ctx.env != ctx.env
            || stored_ctx.tenant_id != ctx.tenant_id
            || Self::normalize_team(stored_ctx) != Self::normalize_team(ctx)
        {
            self.remove_scope_entry(&scope_key);
            self.remove_from_user_waits(&UserLookupKey::from_ctx(ctx, user_id), &entry.session_key);
            return Ok(None);
        }
        if let Some(stored_user) = Self::normalize_user(stored_ctx)
            && stored_user != user_id
        {
            self.remove_scope_entry(&scope_key);
            self.remove_from_user_waits(&UserLookupKey::from_ctx(ctx, user_id), &entry.session_key);
            return Ok(None);
        }
        Ok(Some(entry.session_key))
    }

    fn list_waits_for_user(
        &self,
        ctx: &TenantCtx,
        user_id: &UserId,
    ) -> SessionResult<Vec<SessionKey>> {
        let lookup = UserLookupKey::from_ctx(ctx, user_id);
        let keys: Vec<SessionKey> = self
            .user_waits
            .read()
            .get(&lookup)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default();
        let mut available = Vec::new();
        for key in keys {
            let Some(data) = self.get_session(&key)? else {
                self.remove_from_user_waits(&lookup, &key);
                continue;
            };
            let stored_ctx = &data.tenant_ctx;
            if stored_ctx.env != ctx.env
                || stored_ctx.tenant_id != ctx.tenant_id
                || Self::normalize_team(stored_ctx) != Self::normalize_team(ctx)
            {
                self.remove_from_user_waits(&lookup, &key);
                continue;
            }
            if let Some(stored_user) = Self::normalize_user(stored_ctx)
                && stored_user != user_id
            {
                self.remove_from_user_waits(&lookup, &key);
                continue;
            }
            available.push(key);
        }
        Ok(available)
    }

    fn clear_wait(
        &self,
        ctx: &TenantCtx,
        user_id: &UserId,
        scope: &ReplyScope,
    ) -> SessionResult<()> {
        let scope_key = Self::scope_lookup_key(ctx, user_id, scope);
        let entry = self.scope_index.write().remove(&scope_key);
        if let Some(entry) = entry {
            self.remove_from_user_waits(&UserLookupKey::from_ctx(ctx, user_id), &entry.session_key);
            self.sessions.write().remove(&entry.session_key);
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

#[derive(Clone)]
struct SessionEntry {
    data: SessionData,
    expires_at: Option<Instant>,
    wait_user: Option<UserLookupKey>,
    scope_key: Option<ScopeLookupKey>,
}

#[derive(Clone)]
struct ScopeEntry {
    session_key: SessionKey,
    expires_at: Option<Instant>,
}

#[derive(Clone, Eq, PartialEq, Hash)]
struct ScopeLookupKey {
    env: EnvId,
    tenant: TenantId,
    team: Option<TeamId>,
    user: UserId,
    scope_hash: String,
}

impl ScopeLookupKey {
    fn from_ctx(ctx: &TenantCtx, user: &UserId, scope: &ReplyScope) -> Self {
        Self {
            env: ctx.env.clone(),
            tenant: ctx.tenant_id.clone(),
            team: ctx.team_id.clone().or_else(|| ctx.team.clone()),
            user: user.clone(),
            scope_hash: scope.scope_hash(),
        }
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
