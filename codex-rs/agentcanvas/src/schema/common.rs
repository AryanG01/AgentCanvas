//! Shared enums and types used across the schema

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Execution status for commands
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ExecutionStatus {
    InProgress,
    Completed,
    Failed,
    Declined,
}

/// Status for patch/file changes
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PatchStatus {
    InProgress,
    Completed,
    Failed,
}

/// Status for MCP tool calls
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum McpStatus {
    InProgress,
    Completed,
    Failed,
}

/// Status for collaborative tool calls
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CollabStatus {
    InProgress,
    Completed,
    Failed,
}

/// Status for individual collaborative agents
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CollabAgentStatus {
    PendingInit,
    Running,
    Completed,
    Errored,
    Shutdown,
    NotFound,
}

/// Type of collaborative tool
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CollabTool {
    SpawnAgent,
    SendInput,
    Wait,
    CloseAgent,
}

/// Kind of file change
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ChangeKind {
    Add,
    Delete,
    Update,
}

/// Output stream type for command execution
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum OutputStream {
    Stdout,
    Stderr,
    Combined,
}

/// Token usage metrics
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TokenUsage {
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub output_tokens: i64,
}
