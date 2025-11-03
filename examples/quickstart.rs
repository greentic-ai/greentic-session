use greentic_session::error::{ErrorCode, GreenticError};
use greentic_session::inmemory::InMemorySessionStore;
use greentic_session::mapping::telegram_update_to_session_key;
use greentic_session::model::{
    OutboxEntry, Session, SessionCursor, SessionId, SessionKey, SessionMeta,
};
use greentic_session::store::SessionStore;
use greentic_types::GResult;
use serde_json::Map;
use time::OffsetDateTime;
use uuid::Uuid;

fn build_session(key: SessionKey, tenant: &str) -> Session {
    Session {
        id: SessionId::new(),
        key,
        cursor: SessionCursor {
            flow_id: "onboarding".into(),
            node_id: "welcome-step".into(),
            wait_reason: Some("awaiting_input".into()),
            outbox_seq: 0,
        },
        meta: SessionMeta {
            tenant_id: tenant.into(),
            team_id: Some("team-alpha".into()),
            user_id: Some("user-123".into()),
            labels: Map::new(),
        },
        outbox: vec![OutboxEntry {
            seq: 1,
            payload_sha256: [0; 32],
            created_at: OffsetDateTime::now_utc(),
        }],
        updated_at: OffsetDateTime::now_utc(),
        ttl_secs: 60,
    }
}

fn run_inmemory_demo() -> GResult<()> {
    println!("== In-memory demo ==");
    let store = InMemorySessionStore::new();
    let key = telegram_update_to_session_key("bot-9001", "chat-42", "user-1");
    let session = build_session(key.clone(), "tenant-demo");

    let cas = store.put(session.clone())?;
    println!("Stored session with CAS {cas:?}");

    if let Some((fetched, fetched_cas)) = store.get(&key)? {
        println!("Fetched session cursor @ {}", fetched.cursor.node_id);

        let mut updated = fetched;
        updated.cursor.outbox_seq += 1;
        updated.outbox.push(OutboxEntry {
            seq: updated.cursor.outbox_seq,
            payload_sha256: [1; 32],
            created_at: OffsetDateTime::now_utc(),
        });

        match store.update_cas(updated, fetched_cas)? {
            Ok(next) => println!("CAS update succeeded -> {next:?}"),
            Err(conflict) => println!("CAS conflict, current token {conflict:?}"),
        }
    }

    store.touch(&key, Some(120))?;
    println!("TTL extended via touch");

    store.delete(&key)?;
    println!("Session removed");

    Ok(())
}

#[cfg(feature = "redis")]
fn run_redis_demo() -> GResult<()> {
    use greentic_session::redis_store::RedisSessionStore;

    let url = match std::env::var("REDIS_URL") {
        Ok(url) => url,
        Err(_) => {
            println!("Skipping Redis demo - REDIS_URL not set");
            return Ok(());
        }
    };

    let client = redis::Client::open(url).map_err(redis_unavailable)?;
    let namespace_id = Uuid::new_v4();
    let namespace = format!("greentic:session:example:{namespace_id}");
    let store = RedisSessionStore::with_namespace(client, namespace);

    let key_id = Uuid::new_v4();
    let key = SessionKey(format!("redis-demo-{key_id}"));
    let session = build_session(key.clone(), "tenant-demo");

    let cas = store.put(session.clone())?;
    println!("Redis session stored with CAS {cas:?}");

    if let Some((fetched, fetched_cas)) = store.get(&key)? {
        let mut updated = fetched;
        updated.cursor.outbox_seq += 1;
        match store.update_cas(updated, fetched_cas)? {
            Ok(next) => println!("Redis CAS update -> {next:?}"),
            Err(conflict) => println!("Redis CAS conflict -> {conflict:?}"),
        }
    }

    store.touch(&key, Some(90))?;
    println!("Redis TTL refreshed");
    store.delete(&key)?;
    println!("Redis session removed");
    Ok(())
}

#[cfg(not(feature = "redis"))]
fn run_redis_demo() -> GResult<()> {
    println!("Redis feature disabled; skipping Redis demo");
    Ok(())
}

fn main() -> GResult<()> {
    run_inmemory_demo()?;
    run_redis_demo()?;
    Ok(())
}

#[cfg(feature = "redis")]
fn redis_unavailable(err: redis::RedisError) -> GreenticError {
    GreenticError::new(ErrorCode::Unavailable, err.to_string())
}
