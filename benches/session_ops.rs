use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};

use criterion::{Criterion, criterion_group, criterion_main};
use greentic_session::SessionStore;
use greentic_session::inmemory::InMemorySessionStore;
use greentic_session::model::{
    OutboxEntry, Session, SessionCursor, SessionId, SessionKey, SessionMeta,
};
use serde_json::Map;
use time::OffsetDateTime;

fn bench_session(key: &str) -> Session {
    Session {
        id: SessionId::new(),
        key: SessionKey(key.to_owned()),
        cursor: SessionCursor {
            flow_id: "bench-flow".into(),
            node_id: "bench-node".into(),
            wait_reason: None,
            outbox_seq: 0,
        },
        meta: SessionMeta {
            tenant_id: "bench-tenant".into(),
            team_id: None,
            user_id: None,
            labels: Map::new(),
        },
        outbox: vec![OutboxEntry {
            seq: 1,
            payload_sha256: [42; 32],
            created_at: OffsetDateTime::now_utc(),
        }],
        updated_at: OffsetDateTime::now_utc(),
        ttl_secs: 60,
    }
}

fn inmemory_benches(c: &mut Criterion) {
    let store = InMemorySessionStore::new();

    c.bench_function("inmemory_put", |b| {
        let mut session = bench_session("bench-put");
        let mut counter = 0u64;
        b.iter(|| {
            counter = counter.wrapping_add(1);
            session.cursor.outbox_seq = counter;
            session.outbox[0].seq = counter;
            let bucket = counter % 16;
            session.key = SessionKey(format!("bench-put-{bucket}"));
            black_box(store.put(session.clone()).expect("put"));
        });
    });

    c.bench_function("inmemory_update_cas", |b| {
        let mut session = bench_session("bench-update");
        let key = SessionKey("bench-update".into());
        session.key = key.clone();
        let mut cas = store.put(session.clone()).expect("initial put");
        let counter = AtomicU64::new(1);

        b.iter(|| {
            let seq = counter.fetch_add(1, Ordering::Relaxed) + 1;
            session.cursor.outbox_seq = seq;
            session.outbox[0].seq = seq;
            match store
                .update_cas(session.clone(), cas)
                .expect("update cas bench")
            {
                Ok(next) => cas = next,
                Err(current) => cas = current,
            }
        });
    });

    c.bench_function("inmemory_get", |b| {
        let session = bench_session("bench-get");
        let key = session.key.clone();
        store.put(session).expect("put for get");
        b.iter(|| {
            black_box(store.get(&key).expect("get bench"));
        });
    });
}

criterion_group!(session_ops, inmemory_benches);
criterion_main!(session_ops);
