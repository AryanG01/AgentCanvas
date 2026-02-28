//! Item types and detail structures

use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use ts_rs::TS;

use super::common::*;

/// Type of work item
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ItemType {
    AgentMessage,
    Reasoning,
    CommandExecution,
    FileChange,
    McpToolCall,
    CollabToolCall,
    WebSearch,
    TodoList,
    Error,
}

/// Item-specific details
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum ItemDetails {
    AgentMessage(AgentMessageDetails),
    Reasoning(ReasoningDetails),
    CommandExecution(CommandExecutionDetails),
    FileChange(FileChangeDetails),
    McpToolCall(McpToolCallDetails),
    CollabToolCall(CollabToolCallDetails),
    WebSearch(WebSearchDetails),
    TodoList(TodoListDetails),
    Error(ErrorDetails),
}

/// Agent message content
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AgentMessageDetails {
    pub text: String,
}

/// Agent reasoning content
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReasoningDetails {
    pub text: String,
}

/// Command execution details
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CommandExecutionDetails {
    pub command: String,
    pub status: ExecutionStatus,
    pub exit_code: Option<i32>,
    pub aggregated_output: String,
}

/// File change details
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FileChangeDetails {
    pub changes: Vec<FileChange>,
    pub status: PatchStatus,
}

/// Individual file change
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FileChange {
    pub path: String,
    pub kind: ChangeKind,
}

/// MCP tool call details
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct McpToolCallDetails {
    pub server: String,
    pub tool: String,
    pub arguments: serde_json::Value,
    pub result: Option<McpResult>,
    pub error: Option<McpError>,
    pub status: McpStatus,
}

/// MCP tool call result
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct McpResult {
    pub content: Vec<serde_json::Value>,
    pub structured_content: Option<serde_json::Value>,
}

/// MCP tool call error
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct McpError {
    pub message: String,
}

/// Collaborative tool call details
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CollabToolCallDetails {
    pub tool: CollabTool,
    pub sender_thread_id: String,
    pub receiver_thread_ids: Vec<String>,
    pub prompt: Option<String>,
    pub agents_states: HashMap<String, CollabAgentState>,
    pub status: CollabStatus,
}

/// State of a collaborative agent
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CollabAgentState {
    pub status: CollabAgentStatus,
    pub message: Option<String>,
}

/// Web search details
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WebSearchDetails {
    pub query: String,
    pub action: Option<String>, // WebSearchAction from protocol
}

/// Todo list details
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TodoListDetails {
    pub items: Vec<TodoItem>,
}

/// Individual todo item
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TodoItem {
    pub text: String,
    pub completed: bool,
}

/// Error details
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ErrorDetails {
    pub message: String,
}
