# Migration Guide

# Migration Guide

## Upgrading to 0.4.3

- **TenantCtx enforcement tightened:** session creation/update now requires env, tenant, and team to
  match the caller context. If a stored session includes a user, callers must present the same user;
  attempts to change or introduce user/team values on update are rejected.
- **find_by_user fence:** lookups honor the caller’s `TenantCtx` strictly; mismatched team/user no
  longer returns results and stale mappings are purged.
- **Redis feature opt-in:** the `redis` feature is no longer enabled by default. Enable it explicitly
  (`--features redis` or `features = [\"redis\"]`) to include the Redis backend.

## Upgrading to 0.4.1

- **Before:** Construct Redis stores by passing a `redis::Client` directly (e.g., `RedisSessionStore::new(client)`), and dependants needed a matching `redis` crate version in their own `Cargo.toml`.
- **After:** Select a backend with the Redis-free `SessionBackendConfig` enum and `create_session_store`. For Redis, pass a URL string:

  ```rust
  use greentic_session::{create_session_store, SessionBackendConfig};

  let store = create_session_store(SessionBackendConfig::RedisUrl(
      "redis://127.0.0.1/",
  ))?;
  ```

- The public API no longer mentions `redis::Client`, so downstream crates do not need to import or align Redis types. The default feature set still enables the Redis backend; disable default features to compile without Redis.
