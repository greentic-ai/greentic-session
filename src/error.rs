use greentic_types::GResult;

pub use greentic_types::{ErrorCode, GreenticError};
pub type SessionResult<T> = GResult<T>;

pub(crate) fn serde_error(err: serde_json::Error) -> GreenticError {
    GreenticError::new(ErrorCode::Internal, err.to_string())
}

#[cfg(feature = "redis")]
pub(crate) fn redis_error(err: redis::RedisError) -> GreenticError {
    GreenticError::new(ErrorCode::Unavailable, err.to_string())
}
