use greentic_session::{SessionBackendConfig, SessionResult, create_session_store};
use greentic_types::{EnvId, FlowId, SessionCursor, SessionData, TenantCtx, TenantId, UserId};

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

fn run_inmemory_demo() -> SessionResult<()> {
    println!("== In-memory session demo ==");
    let store = create_session_store(SessionBackendConfig::InMemory)?;
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

fn main() -> SessionResult<()> {
    run_inmemory_demo()
}
