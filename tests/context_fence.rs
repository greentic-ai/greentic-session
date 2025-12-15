use greentic_session::{SessionBackendConfig, create_session_store};
use greentic_types::{
    EnvId, ErrorCode, FlowId, SessionCursor, SessionData, TeamId, TenantCtx, TenantId, UserId,
};

fn ctx(team: &str, user: Option<&str>) -> TenantCtx {
    let env = EnvId::try_from("dev").expect("env id");
    let tenant = TenantId::try_from("tenant-a").expect("tenant id");
    let team_id = TeamId::try_from(team).expect("team id");
    let ctx = TenantCtx::new(env, tenant).with_team(Some(team_id));
    match user {
        Some(user) => {
            let user_id = UserId::try_from(user).expect("user id");
            ctx.with_user(Some(user_id))
        }
        None => ctx.with_user(None),
    }
}

fn data(ctx: &TenantCtx) -> SessionData {
    SessionData {
        tenant_ctx: ctx.clone(),
        flow_id: FlowId::try_from("flow.ctx").expect("flow id"),
        cursor: SessionCursor::new("node.start".to_string()),
        context_json: "{}".into(),
    }
}

#[test]
fn create_session_rejects_team_mismatch() {
    let store = create_session_store(SessionBackendConfig::InMemory)
        .expect("factory should build in-memory store");
    let caller_ctx = ctx("team-a", Some("user-1"));
    let stored_ctx = ctx("team-b", Some("user-1"));
    let err = store
        .create_session(&caller_ctx, data(&stored_ctx))
        .expect_err("team mismatch should be rejected");
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert!(
        err.message.contains("team"),
        "mismatch reason should mention team: {}",
        err.message
    );
}

#[test]
fn update_rejects_context_changes() {
    let store = create_session_store(SessionBackendConfig::InMemory)
        .expect("factory should build in-memory store");
    let base_ctx = ctx("team-a", Some("user-1"));
    let mut stored = data(&base_ctx);
    let key = store
        .create_session(&base_ctx, stored.clone())
        .expect("create succeeds");

    stored.tenant_ctx = stored
        .tenant_ctx
        .clone()
        .with_team(Some(TeamId::try_from("team-b").expect("team id")));
    let err = store
        .update_session(&key, stored)
        .expect_err("team change should be rejected");
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

#[test]
fn find_by_user_enforces_scope_and_user() {
    let store = create_session_store(SessionBackendConfig::InMemory)
        .expect("factory should build in-memory store");
    let base_ctx = ctx("team-a", Some("user-1"));
    let key = store
        .create_session(&base_ctx, data(&base_ctx))
        .expect("create succeeds");

    // Wrong team should not retrieve the session.
    let other_team_ctx = ctx("team-b", Some("user-1"));
    let found = store
        .find_by_user(
            &other_team_ctx,
            other_team_ctx.user_id.as_ref().expect("user present"),
        )
        .expect("lookup")
        .is_some();
    assert!(!found, "lookup should respect team boundary");

    // Wrong user should not retrieve the session.
    let other_user = UserId::try_from("user-2").expect("user id");
    assert!(
        store
            .find_by_user(&base_ctx, &other_user)
            .expect("lookup")
            .is_none(),
        "lookup should respect user binding"
    );

    // Happy path still returns the session.
    let found = store
        .find_by_user(&base_ctx, base_ctx.user_id.as_ref().expect("user present"))
        .expect("lookup")
        .expect("should find session");
    assert_eq!(found.0, key);
}
