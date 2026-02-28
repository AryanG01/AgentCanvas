//! AgentCanvas Event Normalization Layer
//!
//! This crate provides a unified normalization layer for events from multiple sources:
//! - `exec --experimental-json` stream
//! - app-server v2 notifications
//! - persisted rollout files
//!
//! All events are transformed into a canonical `NormalizedEvent` representation
//! that can be consumed by downstream summarization and storage layers.

// pub mod adapters; // temporarily disabled - pre-existing compilation errors
pub mod bridge;
pub mod schema;
pub mod summary;
pub mod utils;

// Re-export public API
pub use schema::{
    common::*, deltas::*, event::*, items::*, EventPayload, EventSource, ItemId,
    NormalizedEvent, ThreadId, TurnId,
};

pub use bridge::{translate as translate_to_app_event, AppEvent};

pub use summary::{
    CommandEvidence, EventReference, LlmSummaryConfig, SessionSummary, SessionSummaryMetadata,
    SessionSummarizer, SummaryEvidence, SummaryNode, SummaryNodeType, TurnLineage,
    SESSION_SUMMARY_SCHEMA_VERSION,
};

pub use utils::{generate_item_id, generate_turn_id};
