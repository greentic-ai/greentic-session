use greentic_types::{GResult, GreenticError};

pub use greentic_types::GreenticError;
pub type SessionResult<T> = GResult<T>;

/// Helper extension trait for mapping backend errors into `GreenticError`.
pub trait IntoGreenticError<T> {
    fn into_gresult(self) -> GResult<T>;
}

impl<T, E> IntoGreenticError<T> for Result<T, E>
where
    GreenticError: From<E>,
{
    fn into_gresult(self) -> GResult<T> {
        self.map_err(GreenticError::from)
    }
}
