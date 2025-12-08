#![forbid(unsafe_code)]

mod backends;

pub mod error;
pub mod inmemory;
pub mod mapping;
pub mod store;

pub use error::{ErrorCode, GreenticError, SessionResult};
pub use greentic_types::{SessionData, SessionKey};
pub use store::SessionStore;

/// Configuration for selecting a session backend.
#[derive(Debug, Clone)]
pub enum SessionBackendConfig {
    /// In-memory store for tests or single-node development.
    InMemory,
    /// Redis-backed store using the default namespace.
    #[cfg(feature = "redis")]
    RedisUrl(String),
    /// Redis-backed store with a custom namespace prefix.
    #[cfg(feature = "redis")]
    RedisUrlWithNamespace { url: String, namespace: String },
}

/// Creates a boxed session store using the provided backend configuration.
pub fn create_session_store(config: SessionBackendConfig) -> SessionResult<Box<dyn SessionStore>> {
    match config {
        SessionBackendConfig::InMemory => Ok(Box::new(inmemory::InMemorySessionStore::new())),
        #[cfg(feature = "redis")]
        SessionBackendConfig::RedisUrl(url) => {
            let store = backends::redis::RedisSessionStore::from_url(&url)?;
            Ok(Box::new(store))
        }
        #[cfg(feature = "redis")]
        SessionBackendConfig::RedisUrlWithNamespace { url, namespace } => {
            let store =
                backends::redis::RedisSessionStore::from_url_with_namespace(&url, namespace)?;
            Ok(Box::new(store))
        }
    }
}
