use greentic_session::ReplyScope;
use greentic_session::inmemory::InMemorySessionStore;
use greentic_session::store::SessionStore;
use greentic_types::{
    EnvId, ErrorCode, FlowId, SessionCursor, SessionData, SessionKey, TenantCtx, TenantId, UserId,
};
use std::thread::sleep;
use std::time::Duration;

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
        pack_id: None,
        cursor: SessionCursor::new(node.to_string()),
        context_json: state.to_string(),
    }
}

fn scope(provider: &str, conversation: &str) -> ReplyScope {
    ReplyScope {
        conversation: format!("{}:{}", provider, conversation),
        thread: None,
        reply_to: None,
        correlation: None,
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

#[test]
fn multiple_waits_are_indexed_and_routed_by_scope() {
    let store = InMemorySessionStore::new();
    let ctx = tenant_ctx("user-77");
    let user = ctx.user_id.as_ref().expect("user present");

    let scope_a = scope("telegram", "chat-a");
    let scope_b = scope("telegram", "chat-b");
    let key_a = SessionKey::new("wait-a");
    let key_b = SessionKey::new("wait-b");

    store
        .register_wait(
            &ctx,
            user,
            &scope_a,
            &key_a,
            sample_data(&ctx, "node.wait.a", "{\"step\":10}"),
            None,
        )
        .expect("wait a registered");
    store
        .register_wait(
            &ctx,
            user,
            &scope_b,
            &key_b,
            sample_data(&ctx, "node.wait.b", "{\"step\":11}"),
            None,
        )
        .expect("wait b registered");

    let waits = store.list_waits_for_user(&ctx, user).expect("list waits");
    assert_eq!(waits.len(), 2);
    assert!(waits.contains(&key_a));
    assert!(waits.contains(&key_b));

    let found_a = store
        .find_wait_by_scope(&ctx, user, &scope_a)
        .expect("find scope a")
        .expect("wait a exists");
    assert_eq!(found_a, key_a);
    let found_b = store
        .find_wait_by_scope(&ctx, user, &scope_b)
        .expect("find scope b")
        .expect("wait b exists");
    assert_eq!(found_b, key_b);
}

#[test]
fn expired_wait_is_not_resumed() {
    let store = InMemorySessionStore::new();
    let ctx = tenant_ctx("user-expire");
    let user = ctx.user_id.as_ref().expect("user present");
    let scope = scope("webchat", "thread-expire");
    let key = SessionKey::new("wait-expire");

    store
        .register_wait(
            &ctx,
            user,
            &scope,
            &key,
            sample_data(&ctx, "node.wait", "{\"step\":99}"),
            Some(Duration::from_millis(30)),
        )
        .expect("wait registered");

    sleep(Duration::from_millis(60));

    let found = store
        .find_wait_by_scope(&ctx, user, &scope)
        .expect("find expired")
        .is_some();
    assert!(!found, "expired wait should not be resumed");

    let waits = store
        .list_waits_for_user(&ctx, user)
        .expect("list waits after expiry");
    assert!(waits.is_empty());
}

#[test]
fn legacy_lookup_is_ambiguous_with_multiple_waits() {
    let store = InMemorySessionStore::new();
    let ctx = tenant_ctx("user-ambiguous");
    let user = ctx.user_id.as_ref().expect("user present");
    let key_a = SessionKey::new("ambiguous-a");
    let key_b = SessionKey::new("ambiguous-b");

    store
        .register_wait(
            &ctx,
            user,
            &scope("slack", "thread-a"),
            &key_a,
            sample_data(&ctx, "node.wait.a", "{\"step\":20}"),
            None,
        )
        .expect("wait a registered");
    store
        .register_wait(
            &ctx,
            user,
            &scope("slack", "thread-b"),
            &key_b,
            sample_data(&ctx, "node.wait.b", "{\"step\":21}"),
            None,
        )
        .expect("wait b registered");

    #[allow(deprecated)]
    let err = store
        .find_by_user(&ctx, user)
        .expect_err("ambiguous waits should error");
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert!(
        err.message.contains("multiple waits"),
        "error should mention ambiguity: {}",
        err.message
    );
}
