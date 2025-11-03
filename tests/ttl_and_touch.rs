use greentic_session::SessionStore;
use greentic_session::inmemory::InMemorySessionStore;
use greentic_session::model::{
    OutboxEntry, Session, SessionCursor, SessionId, SessionKey, SessionMeta,
};
use serde_json::Map;
use std::{thread::sleep, time::Duration};
use time::OffsetDateTime;

fn sample_session(key: &str, ttl_secs: u32) -> Session {
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
            tenant_id: "tenant-a".into(),
            team_id: Some("team-1".into()),
            user_id: Some("user-1".into()),
            labels: Map::new(),
        },
        outbox: vec![OutboxEntry {
            seq: 1,
            payload_sha256: [0; 32],
            created_at: OffsetDateTime::now_utc(),
        }],
        updated_at: OffsetDateTime::now_utc(),
        ttl_secs,
    }
}

#[test]
fn inmemory_touch_extends_ttl() {
    let store = InMemorySessionStore::new();
    let session = sample_session("inmemory-touch", 1);
    let key = session.key.clone();

    store.put(session).expect("put");
    sleep(Duration::from_millis(500));

    assert!(store.touch(&key, Some(3)).expect("touch"));

    sleep(Duration::from_millis(1500));
    assert!(store.get(&key).expect("get").is_some());

    sleep(Duration::from_millis(2000));
    assert!(store.get(&key).expect("get").is_none());
}

#[cfg(feature = "redis")]
mod redis_ttl {
    use super::*;
    use greentic_session::SessionStore;
    use greentic_session::redis_store::RedisSessionStore;
    use uuid::Uuid;

    fn redis_store() -> Option<RedisSessionStore> {
        let url = std::env::var("REDIS_URL").ok()?;
        let client = redis::Client::open(url).ok()?;
        let namespace_id = Uuid::new_v4();
        let namespace = format!("greentic:session:test:{namespace_id}");
        Some(RedisSessionStore::with_namespace(client, namespace))
    }

    #[test]
    fn redis_expiry_respected() {
        let Some(store) = redis_store() else {
            eprintln!("Skipping redis_expiry_respected - REDIS_URL not set or invalid");
            return;
        };

        let mut session = sample_session("redis-touch", 1);
        let key_id = Uuid::new_v4();
        session.key = SessionKey(format!("redis-touch-{key_id}"));
        session.meta.tenant_id = "tenant-redis".into();

        let key = session.key.clone();

        store.put(session).expect("put redis");
        sleep(Duration::from_millis(500));
        assert!(store.touch(&key, Some(3)).expect("touch redis"));

        sleep(Duration::from_millis(1500));
        assert!(store.get(&key).expect("get redis").is_some());

        sleep(Duration::from_millis(2000));
        assert!(store.get(&key).expect("get redis").is_none());
    }
}
