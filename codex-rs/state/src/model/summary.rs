use serde_json::Value;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionSummaryArtifact {
    pub summary_id: String,
    pub thread_id: String,
    pub session_id: String,
    pub schema_version: String,
    pub summary_path: PathBuf,
    pub root_node_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug)]
pub struct SessionSummaryPersistParams {
    pub summary_id: Option<String>,
    pub thread_id: String,
    pub session_id: String,
    pub schema_version: String,
    pub root_node_id: Option<String>,
    pub summary: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SessionSummaryNodeMatch {
    pub summary_id: String,
    pub thread_id: String,
    pub session_id: String,
    pub summary_path: PathBuf,
    pub node_id: String,
    pub parent_node_id: Option<String>,
    pub node_type: String,
    pub title: Option<String>,
    pub node: Value,
    pub matched_file_path: Option<String>,
    pub matched_command: Option<String>,
    pub matched_error_text: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SessionSummarySemanticNodeMatch {
    pub node_match: SessionSummaryNodeMatch,
    pub semantic_score: f64,
}
