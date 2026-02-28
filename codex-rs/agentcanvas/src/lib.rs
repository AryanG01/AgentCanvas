//! AgentCanvas Event Normalization Layer
//!
//! This crate provides a unified normalization layer for events from multiple sources:
//! - `exec --experimental-json` stream
//! - app-server v2 notifications
//! - persisted rollout files
//!
//! All events are transformed into a canonical `NormalizedEvent` representation
//! that can be consumed by downstream summarization and storage layers.

pub mod adapters;
pub mod schema;
pub mod utils;

// Re-export public API
pub use schema::{
    common::*, deltas::*, event::*, items::*, EventPayload, EventSource, ItemId,
    NormalizedEvent, ThreadId, TurnId,
};

pub use adapters::{
    app_server_v2::AppServerV2Adapter, exec_json::ExecJsonAdapter,
    rollout_replay::RolloutReplayAdapter, AdapterError, EventAdapter,
};

pub use utils::{generate_item_id, generate_turn_id};
