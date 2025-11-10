use greentic_session::error::{ErrorCode, GreenticError};
use greentic_session::inmemory::InMemorySessionStore;
use greentic_session::store::SessionStore;
use greentic_types::{
    EnvId, FlowId, GResult, SessionCursor, SessionData, TenantCtx, TenantId, UserId,
};

fn build_ctx(user: &str) -> TenantCtx {
    let env = EnvId::try_from("dev").expect("env");
    let tenant = TenantId::try_from("tenant-demo").expect("tenant");
    let user_id = UserId::try_from(user).expect("user id");
    TenantCtx::new(env, tenant).with_user(Some(user_id))
}

fn build_session(ctx: &TenantCtx, cursor: &str, context_json: &str) -> SessionData {
    SessionData {
        tenant_ctx: ctx.clone(),
        flow_id: FlowId::try_from("onboarding.flow").expect("flow"),
        cursor: SessionCursor::new(cursor.to_string()),
        context_json: context_json.to_string(),
    }
}

fn run_inmemory_demo() -> GResult<()> {
    println!("== In-memory session demo ==");
    let store = InMemorySessionStore::new();
    let ctx = build_ctx("user-123");
    let session = build_session(&ctx, "node.start", "{\"step\":1}");

    let key = store.create_session(&ctx, session.clone())?;
    println!("Created session {}", key.as_str());

    if let Some(data) = store.get_session(&key)? {
        println!("Loaded context payload: {}", data.context_json);
    }

    if let Some((_key, data)) = store.find_by_user(&ctx, ctx.user_id.as_ref().unwrap())? {
        println!(
            "User lookup found cursor {}",
            data.cursor.node_pointer.as_str()
        );
    }

    let updated = build_session(&ctx, "node.wait_input", "{\"step\":2}");
    store.update_session(&key, updated)?;
    println!("Session updated");

    store.remove_session(&key)?;
    println!("Session removed");
    Ok(())
}

fn main() -> GResult<()> {
    run_inmemory_demo()
}

#[allow(dead_code)]
fn redis_unavailable(err: redis::RedisError) -> GreenticError {
    GreenticError::new(ErrorCode::Unavailable, err.to_string())
}
