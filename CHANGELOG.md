# Changelog

## 0.4.1
- Public API no longer exposes Redis types; constructors now take URL strings and Redis is fully internal.
- Added `SessionBackendConfig` + `create_session_store` helper for backend selection without touching Redis clients.
- `SessionStore` uses the crate-local `SessionResult` alias; factory and quickstart docs updated.
