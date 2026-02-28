//! AppEvent types for Engineer 4's UI WebSocket protocol.
//!
//! These event types are the contract between the backend event stream
//! and the frontend React/TypeScript UI. Field names use camelCase via
//! explicit `#[serde(rename)]` to match the TypeScript interfaces in
//! `agentcanvas-ui/src/lib/types.ts`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// UI-facing event types sent over WebSocket to the frontend.
///
/// Each variant maps to a specific UI component update. The enum is
/// internally tagged so the JSON shape is `{ "type": "TurnStarted", ... }`.
///
/// Variant names are PascalCase to match the TypeScript `AppEvent.type`
/// discriminant values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum AppEvent {
    /// Emitted once per session/thread. Creates the root graph node.
    ThreadStarted {
        #[serde(rename = "threadId")]
        thread_id: String,
        ts: i64,
    },

    /// A new turn has begun.
    TurnStarted {
        #[serde(rename = "turnId")]
        turn_id: String,
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "userPrompt")]
        user_prompt: String,
        ts: i64,
    },

    /// A turn has completed or failed.
    TurnComplete {
        #[serde(rename = "turnId")]
        turn_id: String,
        /// One of: "success", "error", "cancelled"
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
        /// One of: "success", "error"
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
        /// One of: "success", "error"
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
