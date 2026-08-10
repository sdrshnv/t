use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Domain(String),

    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("cannot determine data directory: set XDG_DATA_HOME or HOME")]
    DataDirectory,

    #[error("invalid editor command: {0}")]
    EditorCommand(String),
}

impl Error {
    pub fn domain(message: impl Into<String>) -> Self {
        Self::Domain(message.into())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
