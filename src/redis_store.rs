use crate::error::{redis_error, serde_error};
use crate::model::{Cas, Session, SessionKey};
use crate::store::SessionStore;
use greentic_types::GResult;
use redis::{Commands, Connection, RedisResult, Script, Value};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

const DEFAULT_NAMESPACE: &str = "greentic:session";
const LOOKUP_SEGMENT: &str = "lookup";

static UPDATE_LUA: &str = r#"
local key = KEYS[1]
local expected = tonumber(ARGV[1])
local payload = ARGV[2]
local ttl = tonumber(ARGV[3])
local new_cas = tonumber(ARGV[4])
local existing = redis.call("GET", key)
if not existing then
  return {0, 0}
end
local doc = cjson.decode(existing)
local current = tonumber(doc.cas or 0)
if current ~= expected then
  return {1, current or 0}
end
redis.call("SET", key, payload)
if ttl and ttl > 0 then
  redis.call("EXPIRE", key, ttl)
else
  redis.call("PERSIST", key)
end
return {2, new_cas}
"#;

#[derive(Clone)]
pub struct RedisSessionStore {
    client: redis::Client,
    namespace: String,
    update_script: Script,
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
            update_script: Script::new(UPDATE_LUA),
        }
    }

    fn connection(&self) -> GResult<Connection> {
        self.client.get_connection().map_err(redis_error)
    }

    fn data_key(&self, tenant_id: &str, key: &SessionKey) -> String {
        format!("{}:{}:{}", self.namespace, tenant_id, key.as_str())
    }

    fn lookup_key(&self, key: &SessionKey) -> String {
        format!("{}:{}:{}", self.namespace, LOOKUP_SEGMENT, key.as_str())
    }

    fn resolve_tenant(&self, conn: &mut Connection, key: &SessionKey) -> GResult<Option<String>> {
        let lookup_key = self.lookup_key(key);
        conn.get(&lookup_key).map_err(redis_error)
    }

    fn load_envelope(&self, conn: &mut Connection, key: &str) -> GResult<Option<SessionEnvelope>> {
        let payload: Option<String> = conn.get(key).map_err(redis_error)?;
        let envelope = payload
            .map(|raw| serde_json::from_str(&raw))
            .transpose()
            .map_err(serde_error)?;
        Ok(envelope)
    }

    fn serialize_envelope(envelope: &SessionEnvelope) -> GResult<String> {
        serde_json::to_string(envelope).map_err(serde_error)
    }

    fn ttl_arg(session: &Session) -> i64 {
        i64::from(session.ttl_secs)
    }

    fn set_payload(
        conn: &mut Connection,
        key: &str,
        payload: &str,
        ttl_secs: u32,
    ) -> RedisResult<()> {
        if ttl_secs > 0 {
            let _: Value = redis::cmd("SET")
                .arg(key)
                .arg(payload)
                .arg("EX")
                .arg(ttl_secs)
                .query(conn)?;
        } else {
            let _: Value = redis::cmd("SET").arg(key).arg(payload).query(conn)?;
        }
        Ok(())
    }

    fn sync_lookup(
        &self,
        conn: &mut Connection,
        key: &SessionKey,
        tenant_id: &str,
        ttl_secs: u32,
    ) -> RedisResult<()> {
        let lookup_key = self.lookup_key(key);
        if ttl_secs > 0 {
            let _: Value = redis::cmd("SET")
                .arg(&lookup_key)
                .arg(tenant_id)
                .arg("EX")
                .arg(ttl_secs)
                .query(conn)?;
        } else {
            let _: Value = redis::cmd("SET")
                .arg(&lookup_key)
                .arg(tenant_id)
                .query(conn)?;
        }
        Ok(())
    }

    fn touch_lookup(
        &self,
        conn: &mut Connection,
        key: &SessionKey,
        ttl_secs: u32,
    ) -> RedisResult<()> {
        let lookup_key = self.lookup_key(key);
        if ttl_secs > 0 {
            let _: i64 = redis::cmd("EXPIRE")
                .arg(&lookup_key)
                .arg(ttl_secs)
                .query(conn)?;
        } else {
            let _: i64 = redis::cmd("PERSIST").arg(&lookup_key).query(conn)?;
        }
        Ok(())
    }

    fn purge_lookup(&self, conn: &mut Connection, key: &SessionKey) {
        let lookup_key = self.lookup_key(key);
        let _ = redis::cmd("DEL").arg(&lookup_key).query::<i64>(conn);
    }
}

impl SessionStore for RedisSessionStore {
    fn get(&self, key: &SessionKey) -> GResult<Option<(Session, Cas)>> {
        let mut conn = self.connection()?;
        let Some(tenant_id) = self.resolve_tenant(&mut conn, key)? else {
            return Ok(None);
        };
        let redis_key = self.data_key(&tenant_id, key);
        if let Some(envelope) = self.load_envelope(&mut conn, &redis_key)? {
            return Ok(Some((envelope.session, Cas::from(envelope.cas))));
        } else {
            self.purge_lookup(&mut conn, key);
        }
        Ok(None)
    }

    fn put(&self, mut session: Session) -> GResult<Cas> {
        let mut conn = self.connection()?;
        let tenant_id = session.tenant_id().to_owned();
        let redis_key = self.data_key(&tenant_id, &session.key);
        let now = OffsetDateTime::now_utc();
        session.updated_at = now;
        session.normalize();

        let existing_cas = self
            .load_envelope(&mut conn, &redis_key)?
            .map(|envelope| Cas::from(envelope.cas).next());
        let cas = existing_cas.unwrap_or_else(Cas::initial);
        let envelope = SessionEnvelope::new(session, cas);
        let payload = Self::serialize_envelope(&envelope)?;
        Self::set_payload(&mut conn, &redis_key, &payload, envelope.session.ttl_secs)
            .map_err(redis_error)?;
        self.sync_lookup(
            &mut conn,
            &envelope.session.key,
            &tenant_id,
            envelope.session.ttl_secs,
        )
        .map_err(redis_error)?;
        Ok(cas)
    }

    fn update_cas(&self, mut session: Session, expected: Cas) -> GResult<Result<Cas, Cas>> {
        let mut conn = self.connection()?;
        let tenant_id = session.tenant_id().to_owned();
        let redis_key = self.data_key(&tenant_id, &session.key);
        let now = OffsetDateTime::now_utc();
        session.updated_at = now;
        session.normalize();

        let new_cas = expected.next();
        let envelope = SessionEnvelope::new(session, new_cas);
        let payload = Self::serialize_envelope(&envelope)?;
        let ttl = Self::ttl_arg(&envelope.session);

        let (status, cas_value): (i64, u64) = self
            .update_script
            .key(redis_key.clone())
            .arg(expected.value() as i64)
            .arg(payload)
            .arg(ttl)
            .arg(new_cas.value() as i64)
            .invoke(&mut conn)
            .map_err(redis_error)?;

        match status {
            0 => {
                self.purge_lookup(&mut conn, &envelope.session.key);
                Ok(Err(Cas::none()))
            }
            1 => Ok(Err(Cas::from(cas_value))),
            2 => {
                self.touch_lookup(&mut conn, &envelope.session.key, envelope.session.ttl_secs)
                    .map_err(redis_error)?;
                Ok(Ok(new_cas))
            }
            _ => Ok(Err(Cas::none())),
        }
    }

    fn delete(&self, key: &SessionKey) -> GResult<bool> {
        let mut conn = self.connection()?;
        let Some(tenant_id) = self.resolve_tenant(&mut conn, key)? else {
            return Ok(false);
        };
        let redis_key = self.data_key(&tenant_id, key);
        let lookup_key = self.lookup_key(key);
        let removed: i64 = conn.del(&redis_key).map_err(redis_error)?;
        conn.del::<_, i64>(&lookup_key).map_err(redis_error)?;
        Ok(removed > 0)
    }

    fn touch(&self, key: &SessionKey, ttl_secs: Option<u32>) -> GResult<bool> {
        let mut conn = self.connection()?;
        let Some(tenant_id) = self.resolve_tenant(&mut conn, key)? else {
            return Ok(false);
        };
        let redis_key = self.data_key(&tenant_id, key);
        let Some(mut envelope) = self.load_envelope(&mut conn, &redis_key)? else {
            self.purge_lookup(&mut conn, key);
            return Ok(false);
        };

        let now = OffsetDateTime::now_utc();
        envelope.session.updated_at = now;
        if let Some(ttl) = ttl_secs {
            envelope.session.ttl_secs = ttl;
        }

        let payload = Self::serialize_envelope(&envelope)?;
        let ttl = Self::ttl_arg(&envelope.session);

        let (status, _): (i64, u64) = self
            .update_script
            .key(redis_key.clone())
            .arg(envelope.cas)
            .arg(payload)
            .arg(ttl)
            .arg(envelope.cas)
            .invoke(&mut conn)
            .map_err(redis_error)?;

        if status == 2 {
            self.touch_lookup(&mut conn, key, envelope.session.ttl_secs)
                .map_err(redis_error)?;
            Ok(true)
        } else {
            if status == 0 {
                self.purge_lookup(&mut conn, key);
            }
            Ok(false)
        }
    }
}

#[derive(Serialize, Deserialize)]
struct SessionEnvelope {
    cas: u64,
    session: Session,
}

impl SessionEnvelope {
    fn new(mut session: Session, cas: Cas) -> Self {
        session.normalize();
        Self {
            cas: cas.value(),
            session,
        }
    }
}
