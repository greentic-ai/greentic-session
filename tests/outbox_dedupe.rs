use greentic_session::inmemory::InMemorySessionStore;
use greentic_session::model::{
    OutboxEntry, Session, SessionCursor, SessionId, SessionKey, SessionMeta,
};
use greentic_session::store::SessionStore;
use serde_json::Map;
use time::OffsetDateTime;
use uuid::Uuid;

fn duplicate_session(key: &str) -> Session {
    let base_entry = OutboxEntry {
        seq: 1,
        payload_sha256: [1; 32],
        created_at: OffsetDateTime::now_utc(),
    };
    let mut outbox = vec![base_entry.clone(), base_entry.clone()];
    outbox.push(OutboxEntry {
        seq: 2,
        payload_sha256: [2; 32],
        created_at: OffsetDateTime::now_utc(),
    });

    Session {
        id: SessionId::new(),
        key: SessionKey(key.to_owned()),
        cursor: SessionCursor {
            flow_id: "flow".into(),
            node_id: "node".into(),
            wait_reason: None,
            outbox_seq: 0,
        },
        meta: SessionMeta {
            tenant_id: "tenant-dedupe".into(),
            team_id: None,
            user_id: None,
            labels: Map::new(),
        },
        outbox,
        updated_at: OffsetDateTime::now_utc(),
        ttl_secs: 30,
    }
}

#[test]
fn inmemory_dedupes_outbox() {
    let store = InMemorySessionStore::new();
    let session = duplicate_session("dedupe-memory");
    let key = session.key.clone();

    let _ = store.put(session).expect("put");
    let (stored, _) = store.get(&key).expect("get").expect("present");
    assert_eq!(stored.outbox.len(), 2, "duplicate entry should be removed");
}

#[cfg(feature = "redis")]
mod redis_checks {
    use super::*;
    use greentic_session::SessionStore;
    use greentic_session::redis_store::RedisSessionStore;

    fn redis_store() -> Option<RedisSessionStore> {
        let url = std::env::var("REDIS_URL").ok()?;
        let client = redis::Client::open(url).ok()?;
        let namespace_id = Uuid::new_v4();
        let namespace = format!("greentic:session:testdedupe:{namespace_id}");
        Some(RedisSessionStore::with_namespace(client, namespace))
    }

    #[test]
    fn redis_dedupes_on_write() {
        let Some(store) = redis_store() else {
            eprintln!("Skipping redis_dedupes_on_write - REDIS_URL not set or invalid");
            return;
        };

        let mut session = duplicate_session("dedupe-redis");
        let key_id = Uuid::new_v4();
        session.key = SessionKey(format!("dedupe-{key_id}"));
        session.meta.tenant_id = "tenant-dedupe".into();

        let key = session.key.clone();
        let _ = store.put(session).expect("put redis");
        let (stored, _) = store.get(&key).expect("get redis").expect("present");
        assert_eq!(
            stored.outbox.len(),
            2,
            "redis should drop duplicate outbox entries"
        );
    }
}
