use std::fmt::Display;

pub type Result<T> = std::result::Result<T, CacheError>;

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("{0}")]
    Message(String),
    #[error("{context}: {source}")]
    Context {
        context: String,
        #[source]
        source: Box<Self>,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    ParseInt(#[from] std::num::ParseIntError),
    #[error(transparent)]
    TryFromInt(#[from] std::num::TryFromIntError),
    #[error(transparent)]
    Utf8(#[from] std::str::Utf8Error),
    #[error(transparent)]
    FromUtf8(#[from] std::string::FromUtf8Error),
    #[error(transparent)]
    Lzma(#[from] lzma_rs::error::Error),
    #[error(transparent)]
    SerdeJson(#[from] serde_json::Error),
}

impl CacheError {
    pub fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }

    fn with_context(self, context: impl Into<String>) -> Self {
        Self::Context {
            context: context.into(),
            source: Box::new(self),
        }
    }
}

pub trait Context<T> {
    fn context(self, context: impl Display) -> Result<T>;
    fn with_context(self, context: impl FnOnce() -> String) -> Result<T>;
}

impl<T, E> Context<T> for std::result::Result<T, E>
where
    CacheError: From<E>,
{
    fn context(self, context: impl Display) -> Result<T> {
        self.map_err(|error| CacheError::from(error).with_context(context.to_string()))
    }

    fn with_context(self, context: impl FnOnce() -> String) -> Result<T> {
        self.map_err(|error| CacheError::from(error).with_context(context()))
    }
}

impl<T> Context<T> for Option<T> {
    fn context(self, context: impl Display) -> Result<T> {
        self.ok_or_else(|| CacheError::message(context.to_string()))
    }

    fn with_context(self, context: impl FnOnce() -> String) -> Result<T> {
        self.ok_or_else(|| CacheError::message(context()))
    }
}

#[macro_export]
macro_rules! cache_bail {
    ($($arg:tt)*) => {
        return Err($crate::error::CacheError::message(format!($($arg)*)))
    };
}
