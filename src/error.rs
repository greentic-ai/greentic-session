pub use greentic_types::{ErrorCode, GreenticError};
use greentic_types::{GResult, SessionKey};
pub type SessionResult<T> = GResult<T>;

#[cfg(feature = "redis")]
pub(crate) fn serde_error(err: serde_json::Error) -> GreenticError {
    GreenticError::new(ErrorCode::Internal, err.to_string())
}

#[cfg(feature = "redis")]
pub(crate) fn redis_error(err: redis::RedisError) -> GreenticError {
    GreenticError::new(ErrorCode::Unavailable, err.to_string())
}

pub(crate) fn invalid_argument(msg: impl Into<String>) -> GreenticError {
    GreenticError::new(ErrorCode::InvalidInput, msg.into())
}

pub(crate) fn not_found(key: &SessionKey) -> GreenticError {
    GreenticError::new(
        ErrorCode::NotFound,
        format!("session {} was not found", key.as_str()),
    )
}
