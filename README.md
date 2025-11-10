# greentic-session

Greentic’s session manager persists flow execution state between user interactions. A session is
represented by `greentic_types::SessionData`, which bundles the tenant context, flow identifier,
cursor, and serialized execution snapshot so a runner can pause a Wasm flow, wait for input, and
resume exactly where it left off.

## Highlights

- **Shared schema** – The crate reuses the `greentic_types` definitions for `SessionKey`,
  `SessionData`, `TenantCtx`, and `UserId`, ensuring every runtime speaks the same language when
  storing or resuming a flow.
- **Pluggable stores** – The `SessionStore` trait exposes a compact CRUD surface
  (`create_session`, `get_session`, `update_session`, `remove_session`, `find_by_user`). There is a
  thread-safe in-memory implementation plus a Redis implementation for multi-process deployments.
- **User routing** – Each store maintains a secondary map from `(env, tenant, team, user)` to a
  `SessionKey` so inbound activities can be routed to the correct paused flow without the caller
  needing to supply the opaque session identifier.
- **Deterministic key helpers** – The `mapping` module offers helpers for deriving stable
  `SessionKey` values from connector payloads (e.g., Telegram update fields or webhook metadata).

## SessionStore API

```rust
use greentic_session::{SessionStore, SessionData};
use greentic_types::{SessionCursor, TenantCtx};

pub trait SessionStore {
    fn create_session(&self, ctx: &TenantCtx, data: SessionData) -> GResult<SessionKey>;
    fn get_session(&self, key: &SessionKey) -> GResult<Option<SessionData>>;
    fn update_session(&self, key: &SessionKey, data: SessionData) -> GResult<()>;
    fn remove_session(&self, key: &SessionKey) -> GResult<()>;
    fn find_by_user(
        &self,
        ctx: &TenantCtx,
        user: &UserId,
    ) -> GResult<Option<(SessionKey, SessionData)>>;
}
```

All stores persist the full `SessionData` blob so the runner can deserialize the exact execution
context it previously saved. When `find_by_user` returns a result, the runner can call
`update_session` with the resumed snapshot (or `remove_session` once the flow completes).

## Quickstart

```rust
use greentic_session::inmemory::InMemorySessionStore;
use greentic_session::SessionStore;
use greentic_types::{EnvId, FlowId, SessionCursor, SessionData, TenantCtx, TenantId, UserId};

fn demo() -> greentic_types::GResult<()> {
    let store = InMemorySessionStore::new();
    let env = EnvId::try_from("dev")?;
    let tenant = TenantId::try_from("tenant-42")?;
    let user = UserId::try_from("user-7")?;
    let ctx = TenantCtx::new(env, tenant).with_user(Some(user.clone()));

    let snapshot = SessionData {
        tenant_ctx: ctx.clone(),
        flow_id: FlowId::try_from("support.flow")?,
        cursor: SessionCursor::new("node.wait_input"),
        context_json: "{\"ticket\":123}".into(),
    };

    let key = store.create_session(&ctx, snapshot.clone())?;
    let hydrated = store.get_session(&key)?.expect("session present");
    assert_eq!(hydrated.cursor.node_pointer, "node.wait_input");

    let (found_key, _) = store.find_by_user(&ctx, &user)?.expect("user session");
    assert_eq!(found_key, key);

    store.remove_session(&key)?;
    Ok(())
}
```

Run the example with `cargo run --example quickstart` to see the same flow end-to-end.

## Choosing a backend

| Feature flag combo | Backend availability | Suggested usage |
| --- | --- | --- |
| `default` (`redis`) | Redis + in-memory | Production runners |
| `--no-default-features --features inmemory` | In-memory only | Tests, single-node dev |
| `--all-features` | Redis + schema docs | CI / documentation generation |

The Redis backend stores each `SessionData` blob as JSON under
`greentic:session:session:{session_key}` and maintains user lookup keys at
`greentic:session:user:{env}:{tenant}:{team}:{user}`. When a session is removed, the lookup entry is
cleared so new activities fall back to creating a fresh session.

## Deterministic Session Keys

- `mapping::telegram_update_to_session_key(bot_id, chat_id, user_id)`
- `mapping::webhook_to_session_key(source, subject, id_hint)`

Both helpers derive a SHA-256 digest and hex-encode it, making it safe to hash stable identifiers
without leaking PII. Use them when the connector should resume the same session even if the runtime
doesn’t issue a `SessionKey` (e.g., webhooks that only provide conversation IDs).

## Development

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test --all-features
```

Redis tests honor the `REDIS_URL` environment variable. If unset, the Redis-specific tests are
skipped automatically.
