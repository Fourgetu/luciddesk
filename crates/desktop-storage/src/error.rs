//! Errors reported by workspace persistence.
use std::{error::Error, fmt};

/// A database failure or invalid persisted workspace value.
#[derive(Debug)]
pub enum StoreError {
    Database(rusqlite::Error),
    InvalidData(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(error) => write!(formatter, "database error: {error}"),
            Self::InvalidData(error) => write!(formatter, "invalid workspace data: {error}"),
        }
    }
}

impl Error for StoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            Self::InvalidData(_) => None,
        }
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Database(value)
    }
}
