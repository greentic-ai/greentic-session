#![cfg(feature = "redis")]

use greentic_session::{SessionBackendConfig, create_session_store};
use greentic_types::{EnvId, FlowId, SessionCursor, SessionData, TenantCtx, TenantId, UserId};

fn ctx(user: &str) -> TenantCtx {
    let env = EnvId::try_from("dev").expect("env id");
    let tenant = TenantId::try_from("tenant-redis").expect("tenant id");
    let user_id = UserId::try_from(user).expect("user id");
    TenantCtx::new(env, tenant).with_user(Some(user_id))
}

#[test]
fn redis_backend_crud_when_url_provided() {
    let url = match std::env::var("REDIS_URL") {
        Ok(val) => val,
        Err(_) => {
            eprintln!("skipping redis_backend_crud_when_url_provided: REDIS_URL not set");
            return;
        }
    };

    let store =
        create_session_store(SessionBackendConfig::RedisUrl(url)).expect("construct redis store");
    let ctx = ctx("user-redis");
    let data = SessionData {
        tenant_ctx: ctx.clone(),
        flow_id: FlowId::try_from("flow.redis").expect("flow"),
        cursor: SessionCursor::new("node.redis.start".to_string()),
        context_json: "{\"step\":1}".into(),
    };

    let key = store
        .create_session(&ctx, data.clone())
        .expect("create succeeds");
    let fetched = store
        .get_session(&key)
        .expect("get succeeds")
        .expect("present");
    assert_eq!(fetched.cursor.node_pointer, data.cursor.node_pointer);

    let updated = SessionData {
        cursor: SessionCursor::new("node.redis.next".to_string()),
        context_json: "{\"step\":2}".into(),
        ..data.clone()
    };
    store
        .update_session(&key, updated.clone())
        .expect("update succeeds");
    let refreshed = store
        .get_session(&key)
        .expect("get after update")
        .expect("present");
    assert_eq!(refreshed.cursor.node_pointer, "node.redis.next");

    let found = store
        .find_by_user(&ctx, ctx.user_id.as_ref().unwrap())
        .expect("lookup")
        .expect("user mapping");
    assert_eq!(found.0, key);

    store.remove_session(&key).expect("remove succeeds");
    let missing = store.get_session(&key).expect("get after delete");
    assert!(missing.is_none());
}
