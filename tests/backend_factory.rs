use greentic_session::{SessionBackendConfig, create_session_store};
use greentic_types::{EnvId, FlowId, SessionCursor, SessionData, TenantCtx, TenantId, UserId};

fn ctx(user: &str) -> TenantCtx {
    let env = EnvId::try_from("dev").expect("env id");
    let tenant = TenantId::try_from("tenant-a").expect("tenant id");
    let user_id = UserId::try_from(user).expect("user id");
    TenantCtx::new(env, tenant).with_user(Some(user_id))
}

#[test]
fn factory_returns_inmemory_store() {
    let store = create_session_store(SessionBackendConfig::InMemory)
        .expect("factory should build in-memory store");
    let ctx = ctx("user-007");
    let data = SessionData {
        tenant_ctx: ctx.clone(),
        flow_id: FlowId::try_from("flow.demo").expect("flow"),
        pack_id: None,
        cursor: SessionCursor::new("node.start".to_string()),
        context_json: "{}".into(),
    };

    let key = store
        .create_session(&ctx, data.clone())
        .expect("create succeeds");
    let fetched = store
        .get_session(&key)
        .expect("get succeeds")
        .expect("session exists");
    assert_eq!(fetched.cursor.node_pointer, data.cursor.node_pointer);
}
