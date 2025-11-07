# greentic-session

Greentic’s session manager provides a multi-tenant coordination layer for connector flows.  
It offers deterministic session key mapping, optimistic concurrency via compare-and-set tokens, outbox deduplication, TTL management, and pluggable persistence backends.

## Crate Highlights

- **Shared model** – Reuses `greentic-types` primitives (`GResult`, `TenantCtx`, etc.) while defining session-centric structs (`Session`, `SessionMeta`, `SessionCursor`, `OutboxEntry`, `Cas`).
- **Backends** – In-memory (`dashmap` + lazy TTL cleanup) is always available; Redis backend is feature-gated (`redis`, default-enabled) and stores data under `greentic:session:{tenant}:{key}`.
- **Deterministic keys** – Helper mappers hash connector payload hints (e.g., Telegram updates, webhooks) to produce stable `SessionKey` values without leaking PII.
- **Concurrency guarantee** – CAS tokens (`Cas`) increment on every successful write, giving last-writer-wins semantics and protecting against racey updates.
- **Outbox dedupe** – Repeated `(seq, payload_sha256)` pairs are ignored in both backends, ensuring idempotent connector hand-offs.

## Choosing a Backend

| Feature flag combo | Backend availability | Suggested usage |
| --- | --- | --- |
| `default` (`redis`) | Redis + in-memory | Production deployments with Redis |
| `--no-default-features --features inmemory` | In-memory only | Tests, single-node dev |
| `--all-features` | Redis + schema export | CI / schema docs |

Enable the Redis backend with the default feature set. In environments without Redis, disable default features and opt into `inmemory`.

## Quickstart

```bash
cargo run --example quickstart
# optionally, export REDIS_URL and re-run to exercise Redis
```

The example walks through:

1. Mapping a Telegram update onto a `SessionKey`.
2. Creating a `Session` and writing it with `SessionStore::put`.
3. Fetching the CAS token and issuing a CAS update with a new outbox entry.
4. Refreshing TTL/`updated_at` via `touch` and finally deleting the session.
5. Running the same flow against Redis (when `REDIS_URL` is configured).

You can adapt the snippet for your flow orchestration logic. A minimal excerpt:

```rust
use greentic_session::{inmemory::InMemorySessionStore, mapping::telegram_update_to_session_key, model::*};
use greentic_session::SessionStore;
use time::OffsetDateTime;

let store = InMemorySessionStore::new();
let key = telegram_update_to_session_key("bot", "chat", "user");
let session = Session {
    id: SessionId::new(),
    key: key.clone(),
    cursor: SessionCursor { flow_id: "onboard".into(), node_id: "start".into(), wait_reason: None, outbox_seq: 0 },
    meta: SessionMeta { tenant_id: "tenant-42".into(), team_id: None, user_id: None, labels: serde_json::Map::new() },
    outbox: vec![],
    updated_at: OffsetDateTime::now_utc(),
    ttl_secs: 60,
};
let cas = store.put(session)?;
// later…
if let Some((mut current, current_cas)) = store.get(&key)? {
    current.cursor.outbox_seq += 1;
    match store.update_cas(current, current_cas)? {
        Ok(next) => println!("updated with cas {next:?}"),
        Err(conflict) => println!("retry with current cas {conflict:?}"),
    }
}
```

## Deterministic Session Keys

- `mapping::telegram_update_to_session_key(bot_id, chat_id, user_id)`
- `mapping::webhook_to_session_key(source, subject, id_hint)`

Both functions derive a SHA-256 digest and encode it as hex. Avoid placing secrets or raw PII in the inputs—hash inputs or substitute stable, non-sensitive identifiers extracted earlier in your pipeline.

## TTL & Cleanup Semantics

- Each `Session` carries an absolute `ttl_secs`. `put` and `update_cas` stamp `updated_at` to `OffsetDateTime::now_utc()` and recompute the expiry.
- In-memory backend performs lazy expiration during read/write/touch operations and periodically scans (no more often than every 60 s) to evict expired entries.
- Redis backend stores the envelope (`session + cas`) in a single JSON blob and keeps a lookup key for tenant resolution. TTL changes are applied atomically via Lua, and lookup keys mirror the data TTL (or are persisted when TTL is zero).
- `touch` refreshes both `updated_at` and TTL; pass `None` to keep the existing TTL but still bump `updated_at`.

## Thread-safety & Performance

- `InMemorySessionStore` uses `DashMap` for lock-free reads and employs a `parking_lot` mutex only for occasional cleanup scheduling.
- `RedisSessionStore` relies on Lua scripts for CAS updates to guarantee atomicity and uses Redis-native expiration for TTL enforcement. A secondary lookup key (`greentic:session:lookup:{hash}`) tracks tenant-to-key mapping.
- Benchmarks (`cargo bench`) provide baseline throughput for `put`, `update_cas`, and `get` on the in-memory backend via Criterion.

## Testing & Linting

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test --all-features
cargo test --no-default-features --features inmemory
```

Redis integration tests (`ttl_and_touch`, `cas_races`, `outbox_dedupe`) require `REDIS_URL`. CI spins up Redis automatically via a service container.

Local Redis for ad-hoc runs:

```bash
docker run --rm -p 6379:6379 redis:7-alpine
export REDIS_URL=redis://localhost:6379
```

## Versioning & Stability

- Crate metadata follows [Semantic Versioning](https://semver.org/). Initial releases start at `0.x` while APIs and models stabilize.
- Publishing is tag-driven (`v*` tags). GitHub Actions handle fmt/clippy/test for pull requests and main pushes.
- Licensing: MIT (see `LICENSE`). When embedding this crate elsewhere, keep license headers aligned.

## Maintenance Notes

- Extend shared surface area (e.g., adding new fields to `Session`) through `greentic-types` first to avoid duplication.
- Additional backends (SQL, DynamoDB, etc.) should live behind new feature flags and reuse the `SessionStore` trait.
- Observe the `TenantCtx` semantics from the next-gen overview—tenant-aware routing is preserved via the lookup key strategy on Redis and via the `SessionMeta` struct for in-memory computations.

## Releases & Publishing

- Crate versions are sourced directly from each `Cargo.toml`.
- Every push to `master` reruns the auto-tag workflow. When a crate’s manifest changes its version and no matching tag exists yet, a git tag `<crate-name>-v<semver>` is created and pushed.
- The publish workflow lints, builds, and tests the full workspace (all features) before invoking `katyo/publish-crates@v2`.
- Publishing is idempotent: if a crate version is already on crates.io, the workflow succeeds without re-uploading.
- Configure the `CARGO_REGISTRY_TOKEN` secret with a crates.io publish token to enable automated releases.

## Local CI checks

Run the aggregated checks before pushing:

```bash
ci/local_check.sh
```

Useful toggles:

- `LOCAL_CHECK_ONLINE=1` - enable network-reliant steps (publish parity).
- `LOCAL_CHECK_STRICT=1` - fail when optional tooling/files are missing instead of skipping.
- `LOCAL_CHECK_VERBOSE=1` - echo every command (helps when debugging failures).

Example:

```bash
LOCAL_CHECK_ONLINE=1 LOCAL_CHECK_STRICT=1 ci/local_check.sh
```

The script mirrors the GitHub Actions workflows (`fmt`, `clippy`, `build`, `test`, and publish packaging checks) while offline by default, so you can reproduce CI locally with predictable output.
