use crate::model::{Cas, Session, SessionKey};
use crate::store::SessionStore;
use dashmap::DashMap;
use greentic_types::GResult;
use parking_lot::Mutex;
use time::{Duration, OffsetDateTime};

struct Entry {
    session: Session,
    cas: Cas,
    expires_at: Option<OffsetDateTime>,
}

impl Entry {
    fn new(session: Session, cas: Cas) -> Self {
        let expires_at = session.expires_at();
        Self {
            session,
            cas,
            expires_at,
        }
    }

    fn is_expired(&self, now: OffsetDateTime) -> bool {
        match self.expires_at {
            Some(exp) => now >= exp,
            None => false,
        }
    }
}

/// In-memory implementation backed by a concurrent hash map.
pub struct InMemorySessionStore {
    entries: DashMap<SessionKey, Entry>,
    cleanup_hint: Mutex<OffsetDateTime>,
}

impl Default for InMemorySessionStore {
    fn default() -> Self {
        Self {
            entries: DashMap::new(),
            cleanup_hint: Mutex::new(OffsetDateTime::now_utc()),
        }
    }
}

impl InMemorySessionStore {
    /// Constructs a store with no background maintenance. Expiration is handled lazily on access.
    pub fn new() -> Self {
        Self::default()
    }

    fn now() -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }

    fn sanitize_for_write(session: &mut Session, now: OffsetDateTime) {
        session.updated_at = now;
        session.normalize();
    }

    fn maybe_cleanup(&self, now: OffsetDateTime) {
        let mut guard = self.cleanup_hint.lock();
        if now - *guard < Duration::seconds(60) {
            return;
        }

        let stale_keys: Vec<_> = self
            .entries
            .iter()
            .filter_map(|entry| {
                if entry.value().is_expired(now) {
                    Some(entry.key().clone())
                } else {
                    None
                }
            })
            .collect();

        for key in stale_keys {
            self.entries.remove(&key);
        }

        *guard = now;
    }
}

impl SessionStore for InMemorySessionStore {
    fn get(&self, key: &SessionKey) -> GResult<Option<(Session, Cas)>> {
        let now = Self::now();
        self.maybe_cleanup(now);
        if let Some(entry) = self.entries.get(key) {
            if entry.is_expired(now) {
                drop(entry);
                self.entries.remove(key);
                return Ok(None);
            }
            return Ok(Some((entry.session.clone(), entry.cas)));
        }
        Ok(None)
    }

    fn put(&self, mut session: Session) -> GResult<Cas> {
        let key = session.key.clone();
        let now = Self::now();
        self.maybe_cleanup(now);
        if let Some(existing) = self.entries.get(&key) {
            if existing.is_expired(now) {
                drop(existing);
                self.entries.remove(&key);
            }
        }

        Self::sanitize_for_write(&mut session, now);

        let mut cas = Cas::initial();
        match self.entries.entry(key) {
            dashmap::mapref::entry::Entry::Occupied(mut occ) => {
                cas = occ.get().cas.next();
                occ.insert(Entry::new(session, cas));
            }
            dashmap::mapref::entry::Entry::Vacant(vac) => {
                vac.insert(Entry::new(session, cas));
            }
        }
        Ok(cas)
    }

    fn update_cas(&self, mut session: Session, expected: Cas) -> GResult<Result<Cas, Cas>> {
        let key = session.key.clone();
        let now = Self::now();
        self.maybe_cleanup(now);
        Self::sanitize_for_write(&mut session, now);

        if let Some(mut guard) = self.entries.get_mut(&key) {
            if guard.is_expired(now) {
                drop(guard);
                self.entries.remove(&key);
                return Ok(Err(Cas::none()));
            }

            let current = guard.cas;
            if current != expected {
                return Ok(Err(current));
            }

            let next = current.next();
            guard.cas = next;
            guard.session = session;
            guard.expires_at = guard.session.expires_at();
            return Ok(Ok(next));
        }
        Ok(Err(Cas::none()))
    }

    fn delete(&self, key: &SessionKey) -> GResult<bool> {
        Ok(self.entries.remove(key).is_some())
    }

    fn touch(&self, key: &SessionKey, ttl_secs: Option<u32>) -> GResult<bool> {
        let now = Self::now();
        self.maybe_cleanup(now);
        if let Some(mut guard) = self.entries.get_mut(key) {
            if guard.is_expired(now) {
                drop(guard);
                self.entries.remove(key);
                return Ok(false);
            }

            if let Some(ttl) = ttl_secs {
                guard.session.ttl_secs = ttl;
            }
            guard.session.updated_at = now;
            guard.session.normalize();
            guard.expires_at = guard.session.expires_at();
            return Ok(true);
        }
        Ok(false)
    }
}
