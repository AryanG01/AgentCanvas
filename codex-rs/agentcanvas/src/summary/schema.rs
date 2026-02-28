//! Session summary schema produced from normalized AgentCanvas events.

use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;

use crate::schema::EventSource;

/// Session summary schema version.
pub const SESSION_SUMMARY_SCHEMA_VERSION: &str = "agentcanvas.session.v1";

/// Compact summary for one thread/session timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SessionSummary {
    pub schema_version: String,
    pub thread_id: String,
    pub root_node_id: String,
    pub metadata: SessionSummaryMetadata,
    pub nodes: Vec<SummaryNode>,
}

/// Session-level summary metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SessionSummaryMetadata {
    pub created_at: i64,
    pub updated_at: i64,
    pub turn_count: u32,
    pub sources: Vec<EventSource>,
}

/// Node types rendered by downstream graph consumers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SummaryNodeType {
    Session,
    Turn,
    Plan,
    Execution,
    CodeChanges,
    ExternalTools,
    Error,
}

impl std::fmt::Display for SummaryNodeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            SummaryNodeType::Session => "session",
            SummaryNodeType::Turn => "turn",
            SummaryNodeType::Plan => "plan",
            SummaryNodeType::Execution => "execution",
            SummaryNodeType::CodeChanges => "code_changes",
            SummaryNodeType::ExternalTools => "external_tools",
            SummaryNodeType::Error => "error",
        };
        write!(f, "{value}")
    }
}

/// Summary node with parent linkage and evidence pointers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SummaryNode {
    pub node_id: String,
    pub parent_id: Option<String>,
    pub node_type: SummaryNodeType,
    pub title: String,
    pub summary: Option<String>,
    pub status: Option<String>,
    pub turn_id: Option<String>,
    pub promoted: bool,
    pub evidence: SummaryEvidence,
    pub lineage: Option<TurnLineage>,
}

/// Turn lineage metadata for forks/rollbacks.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TurnLineage {
    pub parent_turn_id: Option<String>,
    pub forked_from_thread_id: Option<String>,
    pub started_after_rollback: bool,
    pub was_rolled_back: bool,
}

/// Evidence pointer collection for a node.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SummaryEvidence {
    pub event_refs: Vec<EventReference>,
    pub file_paths: Vec<String>,
    pub commands: Vec<CommandEvidence>,
    pub external_tools: Vec<String>,
    pub errors: Vec<String>,
}

/// Event pointer so summaries can map to source evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EventReference {
    pub source: EventSource,
    pub source_event_type: String,
    pub timestamp: i64,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
}

/// Command evidence pointer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CommandEvidence {
    pub command: String,
    pub exit_code: Option<i32>,
}
