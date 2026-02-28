//! Event adapter trait definition

use futures::Stream;
use thiserror::Error;

use crate::schema::{EventSource, NormalizedEvent};

/// Error types for event adapters
#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("Failed to parse event: {message}")]
    ParseError {
        message: String,
        raw_input: String,
    },

    #[error("Stream closed unexpectedly")]
    StreamClosed,

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),
}

/// Event adapter trait for streaming normalized events
pub trait EventAdapter: Stream<Item = Result<NormalizedEvent, AdapterError>> + Unpin {
    /// Get the event source this adapter handles
    fn source(&self) -> EventSource;
}
