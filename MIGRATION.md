# Migration Guide

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
