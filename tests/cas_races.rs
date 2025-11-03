use greentic_session::inmemory::InMemorySessionStore;
use greentic_session::model::{
    OutboxEntry, Session, SessionCursor, SessionId, SessionKey, SessionMeta,
};
use greentic_session::store::SessionStore;
use serde_json::Map;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

fn base_session(key_suffix: &str) -> Session {
    Session {
        id: SessionId::new(),
        key: SessionKey(format!("cas-{key_suffix}")),
        cursor: SessionCursor {
            flow_id: "flow".into(),
            node_id: "node".into(),
            wait_reason: None,
            outbox_seq: 0,
        },
        meta: SessionMeta {
            tenant_id: "tenant-cas".into(),
            team_id: None,
            user_id: None,
            labels: Map::new(),
        },
        outbox: vec![],
        updated_at: OffsetDateTime::now_utc(),
        ttl_secs: 30,
    }
}

fn new_outbox(seq: u64) -> OutboxEntry {
    let mut hasher = Sha256::new();
    hasher.update(seq.to_be_bytes());
    let digest = hasher.finalize();
    let mut payload = [0u8; 32];
    payload.copy_from_slice(&digest);
    OutboxEntry {
        seq,
        payload_sha256: payload,
        created_at: OffsetDateTime::now_utc(),
    }
}

#[test]
fn inmemory_cas_conflict_propagates_latest() {
    let store = InMemorySessionStore::new();
    let session = base_session("memory");
    let key = session.key.clone();

    let cas = store.put(session.clone()).expect("initial put");
    let (snapshot, current) = store.get(&key).expect("get").expect("present");
    assert_eq!(cas, current);

    let mut writer_a = snapshot.clone();
    writer_a.cursor.outbox_seq = 1;
    writer_a.outbox.push(new_outbox(1));

    let cas_a = store
        .update_cas(writer_a.clone(), current)
        .expect("update cas a")
        .expect("cas match");

    let mut writer_b = snapshot;
    writer_b.cursor.outbox_seq = 2;
    writer_b.outbox.push(new_outbox(2));

    match store.update_cas(writer_b, current).expect("update cas b") {
        Ok(_) => panic!("writer B update should have failed due to CAS mismatch"),
        Err(current_cas) => assert_eq!(current_cas, cas_a),
    }

    let (final_session, final_cas) = store.get(&key).expect("get").expect("present");
    assert_eq!(final_cas, cas_a);
    assert_eq!(final_session.cursor.outbox_seq, 1);
}

#[cfg(feature = "redis")]
mod redis_cases {
    use super::*;
    use greentic_session::SessionStore;
    use greentic_session::redis_store::RedisSessionStore;

    fn redis_store() -> Option<RedisSessionStore> {
        let url = std::env::var("REDIS_URL").ok()?;
        let client = redis::Client::open(url).ok()?;
        let namespace_id = Uuid::new_v4();
        let namespace = format!("greentic:session:testcas:{namespace_id}");
        Some(RedisSessionStore::with_namespace(client, namespace))
    }

    #[test]
    fn redis_cas_conflict() {
        let Some(store) = redis_store() else {
            eprintln!("Skipping redis_cas_conflict - REDIS_URL not set or invalid");
            return;
        };

        let mut session = base_session("redis");
        let key_id = Uuid::new_v4();
        session.key = SessionKey(format!("cas-{key_id}"));
        session.meta.tenant_id = "tenant-cas".into();

        let key = session.key.clone();
        let cas = store.put(session.clone()).expect("put redis");
        let (snapshot, current) = store.get(&key).expect("get redis").expect("present");
        assert_eq!(cas, current);

        let mut writer_a = snapshot.clone();
        writer_a.cursor.outbox_seq = 10;
        writer_a.outbox.push(new_outbox(10));

        let cas_a = store
            .update_cas(writer_a.clone(), current)
            .expect("update cas a redis")
            .expect("cas match redis");

        let mut writer_b = snapshot;
        writer_b.cursor.outbox_seq = 20;
        writer_b.outbox.push(new_outbox(20));

        match store
            .update_cas(writer_b, current)
            .expect("update cas b redis")
        {
            Ok(_) => panic!("writer B redis should fail due to CAS mismatch"),
            Err(current_cas) => assert_eq!(current_cas, cas_a),
        }

        let (final_session, final_cas) = store.get(&key).expect("get redis").expect("present");
        assert_eq!(final_cas, cas_a);
        assert_eq!(final_session.cursor.outbox_seq, 10);
    }
}
