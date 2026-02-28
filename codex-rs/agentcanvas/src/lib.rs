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
pub use schema::EventPayload;
pub use schema::EventSource;
pub use schema::ItemId;
pub use schema::NormalizedEvent;
pub use schema::ThreadId;
pub use schema::TurnId;
pub use schema::common::*;
pub use schema::deltas::*;
pub use schema::event::*;
pub use schema::items::*;

pub use bridge::AppEvent;
pub use bridge::translate as translate_to_app_event;

pub use summary::CommandEvidence;
pub use summary::EventReference;
pub use summary::LlmSummaryConfig;
pub use summary::SESSION_SUMMARY_SCHEMA_VERSION;
pub use summary::SessionSummarizer;
pub use summary::SessionSummary;
pub use summary::SessionSummaryMetadata;
pub use summary::SummaryEvidence;
pub use summary::SummaryNode;
pub use summary::SummaryNodeType;
pub use summary::TurnLineage;

pub use utils::generate_item_id;
pub use utils::generate_turn_id;
