//! Core normalized event schema

use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;

use super::common::TokenUsage;
use super::deltas::DeltaDetails;
use super::items::ItemDetails;
use super::items::ItemType;

/// Thread identifier (UUID string)
pub type ThreadId = String;

/// Turn identifier
pub type TurnId = String;

/// Item identifier
pub type ItemId = String;

/// Source of the event
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum EventSource {
    ExecJson,
    AppServerV2,
    RolloutReplay,
}

/// Normalized event envelope with universal metadata
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NormalizedEvent {
    /// Thread identifier
    pub thread_id: ThreadId,

    /// Turn identifier (None for thread-level events)
    pub turn_id: Option<TurnId>,

    /// Item identifier (None for turn/thread-level events)
    pub item_id: Option<ItemId>,

    /// Unix timestamp in seconds
    pub timestamp: i64,

    /// Schema version (starts at 1)
    pub schema_version: u32,

    /// Event source for provenance tracking
    pub source: EventSource,

    /// Original event type string for debugging
    pub source_event_type: String,

    /// Event payload
    pub payload: EventPayload,
}

/// Event payload with lifecycle semantics
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "event_type", rename_all = "snake_case")]
#[ts(export)]
pub enum EventPayload {
    // Thread lifecycle
    ThreadStarted(ThreadStartedPayload),
    ThreadClosed,

    // Turn lifecycle
    TurnStarted(TurnStartedPayload),
    TurnCompleted(TurnCompletedPayload),
    TurnFailed(TurnFailedPayload),

    // Item lifecycle
    ItemStarted(ItemStartedPayload),
    ItemUpdated(ItemUpdatedPayload),
    ItemCompleted(ItemCompletedPayload),

    // Streaming deltas
    ItemDelta(ItemDeltaPayload),

    // Errors
    Error(ErrorPayload),
}

/// Thread started payload
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ThreadStartedPayload {
    // No additional fields beyond envelope
}

/// Turn started payload
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TurnStartedPayload {
    // No additional fields beyond envelope
}

/// Turn completed payload
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TurnCompletedPayload {
    pub usage: TokenUsage,
}

/// Turn failed payload
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TurnFailedPayload {
    pub error_message: String,
}

/// Item started payload
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ItemStartedPayload {
    pub item_type: ItemType,
    pub details: ItemDetails,
}

/// Item updated payload
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ItemUpdatedPayload {
    pub item_type: ItemType,
    pub details: ItemDetails,
}

/// Item completed payload
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ItemCompletedPayload {
    pub item_type: ItemType,
    pub details: ItemDetails,
}

/// Item delta payload for streaming
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ItemDeltaPayload {
    pub item_type: ItemType,
    pub delta: DeltaDetails,
}

/// Error payload
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ErrorPayload {
    pub message: String,
    pub code: Option<String>,
}

impl NormalizedEvent {
    /// Current schema version
    pub const SCHEMA_VERSION: u32 = 1;

    /// Create a new NormalizedEvent with current schema version
    pub fn new(
        thread_id: ThreadId,
        turn_id: Option<TurnId>,
        item_id: Option<ItemId>,
        timestamp: i64,
        source: EventSource,
        source_event_type: String,
        payload: EventPayload,
    ) -> Self {
        Self {
            thread_id,
            turn_id,
            item_id,
            timestamp,
            schema_version: Self::SCHEMA_VERSION,
            source,
            source_event_type,
            payload,
        }
    }
}
