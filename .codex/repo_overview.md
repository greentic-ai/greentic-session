# Repository Overview

## 1. High-Level Purpose
- Greentic Session is a Rust crate that persists multi-tenant flow execution state so Greentic runtimes can pause and resume Wasm flows. It exposes a `SessionStore` trait with pluggable backends and deterministic key helpers for routing user activities.
- Domains/tech: Rust 2024 crate, optional Redis integration hidden behind Redis-free APIs, in-memory store for tests/single-node use, serde/JSON for payloads, UUID-based session keys. Public surface is now Redis-agnostic via `SessionBackendConfig` + `create_session_store`.

## 2. Main Components and Functionality
- **Path:** `src/store.rs`  
  **Role:** Core `SessionStore` trait defining CRUD operations plus user-based lookup for active sessions. Returns the crate-local `SessionResult` alias for shared error handling.  
  **Key functionality:** Create, fetch, update, remove sessions; find sessions by tenant/user context to route inbound events.  
  **Key dependencies / integration points:** Uses `greentic_types` for `SessionData`, `SessionKey`, `TenantCtx`, and `UserId`.
- **Path:** `src/inmemory.rs`  
  **Role:** Thread-safe in-memory `SessionStore` implementation.  
  **Key functionality:** Generates UUID-based keys; validates tenant context alignment; stores sessions in `HashMap`; maintains secondary user lookup map; purges stale mappings on update/remove.  
  **Key dependencies / integration points:** Relies on `parking_lot::RwLock` for concurrency; uses `greentic_types` tenant/user IDs and session payloads.
- **Path:** `src/backends/redis.rs` (feature `redis`)  
  **Role:** Redis-backed `SessionStore` mirroring in-memory semantics; Redis types are internal.  
  **Key functionality:** Generates UUID session keys; serializes session payloads to JSON; stores blobs under `namespace:session:{key}`; maintains user lookup keys under `namespace:user:{env}:{tenant}:{team}:{user}`; cleans up mappings on updates/removals and guards against mismatched tenant contexts. Constructors accept Redis URLs (and optional namespace) only.  
  **Key dependencies / integration points:** Uses `redis` client connections and `serde_json`; default namespace `greentic:session`; constructed via `SessionBackendConfig`.
- **Path:** `src/lib.rs`  
  **Role:** Crate entrypoint exposing Redis-free public API.  
  **Key functionality:** Re-exports `SessionStore`, `SessionKey`, `SessionData`, error types; provides `SessionBackendConfig` enum and `create_session_store` factory to select in-memory or Redis backend without exposing Redis types.
- **Path:** `src/mapping.rs`  
  **Role:** Deterministic helpers to derive `SessionKey` values from external event identifiers.  
  **Key functionality:** SHA-256 + hex hashing for Telegram updates (`tg:{bot}:{chat}:{user}`) and generic webhooks (`wh:{source}:{subject}:{id}`); unit tests ensure stability.  
  **Key dependencies / integration points:** Uses `sha2` and `hex`; returns `SessionKey` from `greentic_types`.
- **Path:** `src/error.rs`  
  **Role:** Error helpers aligning store errors with shared `greentic_types` error codes.  
  **Key functionality:** Builders for serde and Redis errors, invalid input, and not-found responses.
- **Path:** `examples/quickstart.rs`  
  **Role:** Demonstrates end-to-end usage via the new `create_session_store` factory with the in-memory backend: create, fetch, user lookup, update, remove with sample tenant context and flow cursor.
- **Path:** `tests/backend_factory.rs`  
  **Role:** Validates the backend factory returns a working in-memory store for CRUD flows.
- **Path:** `tests/redis_backend.rs` (feature `redis`)  
  **Role:** Integration test for the Redis backend via `SessionBackendConfig::RedisUrl`; runs only when `REDIS_URL` is set.
- **Path:** `tests/`  
  **Role:** Integration and property tests for store semantics and deterministic mappings.  
  **Key functionality:** In-memory CRUD + user lookup flow; determinism and input-sensitivity for mapping helpers using proptest.

## 3. Work In Progress, TODOs, and Stubs
- None identified in code (binary target removed).

## 4. Broken, Failing, or Conflicting Areas
- No failing tests or build errors observed on Rust 1.89.0. `cargo clippy --all-targets --all-features` and `cargo test --all-features` complete successfully (Redis integration test runs when `REDIS_URL` is set).

## 5. Notes for Future Work
- Consider documenting schema (`schema` feature) or interface integration (`interfaces` feature) behavior if added in the future.
- Expand backend factory to support additional storage providers beyond Redis and in-memory.
