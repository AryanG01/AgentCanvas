//! AppEvent types for Engineer 4's UI WebSocket protocol.
//!
//! These 6 event types are the contract between the backend event stream
//! and the frontend React/TypeScript UI. Field names use camelCase to
//! match the TypeScript interface.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// UI-facing event types sent over WebSocket to the frontend.
///
/// Each variant maps to a specific UI component update. The enum is
/// adjacently tagged so the JSON shape is `{ "type": "TurnStarted", ... }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum AppEvent {
    /// A new turn has begun.
    TurnStarted {
        #[serde(rename = "turnId")]
        turn_id: String,
        #[serde(rename = "sessionId")]
        session_id: String,
        ts: i64,
    },

    /// A turn has completed or failed.
    TurnComplete {
        #[serde(rename = "turnId")]
        turn_id: String,
        status: String,
        ts: i64,
    },

    /// A shell command was executed.
    CommandExecution {
        id: String,
        #[serde(rename = "turnId")]
        turn_id: String,
        cmd: String,
        #[serde(rename = "exitCode")]
        exit_code: Option<i32>,
        stdout: String,
        stderr: String,
        ts: i64,
    },

    /// An MCP tool was invoked.
    McpToolCall {
        id: String,
        #[serde(rename = "turnId")]
        turn_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        input: serde_json::Value,
        output: Option<serde_json::Value>,
        status: String,
        ts: i64,
    },

    /// A file patch was applied.
    PatchApply {
        id: String,
        #[serde(rename = "turnId")]
        turn_id: String,
        filename: String,
        diff: String,
        status: String,
        ts: i64,
    },

    /// The agent's plan/todo list was updated.
    PlanUpdate {
        id: String,
        #[serde(rename = "turnId")]
        turn_id: String,
        text: String,
        ts: i64,
    },
}
