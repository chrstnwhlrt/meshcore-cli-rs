//! Error types for the CLI.

use thiserror::Error;

/// CLI error type.
#[derive(Debug, Error)]
pub enum CliError {
    /// Device connection error.
    #[error("Connection error: {0}")]
    Connection(#[from] meshcore::Error),

    /// Serial port error.
    #[error("Serial port error: {0}")]
    Serial(String),

    /// Command error.
    #[error("Command error: {0}")]
    Command(String),

    /// Contact not found.
    #[error("Contact not found: {0}")]
    ContactNotFound(String),

    /// Channel not found.
    #[error("Channel not found: {0}")]
    ChannelNotFound(String),

    /// Invalid argument.
    #[error("Invalid argument: {0}")]
    InvalidArgument(String),

    /// Timeout.
    #[error("Timeout waiting for {0}")]
    Timeout(String),

    /// IO error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Script error.
    #[error("Script error at line {line}: {message}")]
    Script { line: usize, message: String },
}

/// Result type for CLI operations.
pub type Result<T> = std::result::Result<T, CliError>;
