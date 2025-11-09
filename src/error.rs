use std::io;

use thiserror::Error;

/// Shared error type for the Tyco parser.
#[derive(Debug, Error)]
pub enum TycoError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("Unknown struct '{0}'")]
    UnknownStruct(String),
    #[error("Reference error: {0}")]
    Reference(String),
}

impl TycoError {
    pub fn parse(msg: impl Into<String>) -> Self {
        TycoError::Parse(msg.into())
    }
}
