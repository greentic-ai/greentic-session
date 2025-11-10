use greentic_session::inmemory::InMemorySessionStore;
use greentic_session::store::SessionStore;
use greentic_types::{EnvId, FlowId, SessionCursor, SessionData, TenantCtx, TenantId, UserId};

fn tenant_ctx(user: &str) -> TenantCtx {
    let env = EnvId::try_from("dev").expect("env id");
    let tenant = TenantId::try_from("tenant-a").expect("tenant id");
    let user_id = UserId::try_from(user).expect("user id");
    TenantCtx::new(env, tenant).with_user(Some(user_id))
}

fn sample_data(ctx: &TenantCtx, node: &str, state: &str) -> SessionData {
    SessionData {
        tenant_ctx: ctx.clone(),
        flow_id: FlowId::try_from("flow-alpha").expect("flow"),
        cursor: SessionCursor::new(node.to_string()),
        context_json: state.to_string(),
    }
}

#[test]
fn create_get_update_remove_flow_session() {
    let store = InMemorySessionStore::new();
    let ctx = tenant_ctx("user-42");
    let data = sample_data(&ctx, "node.enter", "{\"step\":1}");

    let key = store
        .create_session(&ctx, data.clone())
        .expect("session created");
    let snapshot = store
        .get_session(&key)
        .expect("get session")
        .expect("data present");
    assert_eq!(snapshot.context_json, data.context_json);

    let found = store
        .find_by_user(&ctx, ctx.user_id.as_ref().unwrap())
        .expect("lookup")
        .expect("user session");
    assert_eq!(found.0, key);

    let mut updated_ctx = ctx.clone();
    updated_ctx = updated_ctx.with_user(ctx.user_id.clone());
    let updated = sample_data(&updated_ctx, "node.wait", "{\"step\":2}");
    store
        .update_session(&key, updated.clone())
        .expect("update succeeds");
    let after = store
        .get_session(&key)
        .expect("get after update")
        .expect("present after update");
    assert_eq!(after.cursor.node_pointer, "node.wait");

    store.remove_session(&key).expect("remove succeeds");
    let missing = store.get_session(&key).expect("get after delete");
    assert!(missing.is_none());
}
