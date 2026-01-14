use thiserror::Error as ThisError;

#[derive(Debug, ThisError)]
pub enum Error {
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("i/o error: {0}")]
    Io(String),
    #[error("EOF")]
    Eof,
}

impl From<serde_json::error::Error> for Error {
    fn from(error: serde_json::error::Error) -> Self {
        Error::Serialization(error.to_string())
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Error::Io(format!("{}", error))
    }
}
