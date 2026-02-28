//! Deterministic summary builder from normalized AgentCanvas events.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::schema::CollabAgentStatus;
use crate::schema::CollabStatus;
use crate::schema::EventPayload;
use crate::schema::EventSource;
use crate::schema::ExecutionStatus;
use crate::schema::ItemDetails;
use crate::schema::McpStatus;
use crate::schema::NormalizedEvent;
use crate::schema::PatchStatus;
use crate::schema::TokenUsage;

use super::schema::CommandEvidence;
use super::schema::EventReference;
use super::schema::SESSION_SUMMARY_SCHEMA_VERSION;
use super::schema::SessionSummary;
use super::schema::SessionSummaryMetadata;
use super::schema::SummaryEvidence;
use super::schema::SummaryNode;
use super::schema::SummaryNodeType;
use super::schema::TurnLineage;

const LARGE_EDIT_FILE_THRESHOLD: usize = 3;
const MAJOR_PLAN_PIVOT_THRESHOLD: usize = 2;

/// Builds a compact summary graph from a stream of normalized events.
#[derive(Default)]
pub struct SessionSummarizer {
    thread_id: Option<String>,
    created_at: Option<i64>,
    updated_at: Option<i64>,
    sources: SourceTracker,
    turns: BTreeMap<String, TurnAccumulator>,
    turn_order: Vec<String>,
}

impl SessionSummarizer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ingest(&mut self, event: &NormalizedEvent) {
        self.thread_id
            .get_or_insert_with(|| event.thread_id.clone());
        self.created_at = Some(self.created_at.map_or(event.timestamp, |created_at| {
            created_at.min(event.timestamp)
        }));
        self.updated_at = Some(self.updated_at.map_or(event.timestamp, |updated_at| {
            updated_at.max(event.timestamp)
        }));
        self.sources.observe(&event.source);

        let Some(turn_id) = event.turn_id.as_deref() else {
            return;
        };

        let event_ref = EventReference {
            source: event.source.clone(),
            source_event_type: event.source_event_type.clone(),
            timestamp: event.timestamp,
            turn_id: event.turn_id.clone(),
            item_id: event.item_id.clone(),
        };

        let turn = self.turn_mut(turn_id, event.timestamp);
        turn.event_refs.push(event_ref.clone());

        match &event.payload {
            EventPayload::TurnStarted(_) => {
                turn.started_at.get_or_insert(event.timestamp);
                if turn.status.is_none() {
                    turn.status = Some("in_progress".to_string());
                }
            }
            EventPayload::TurnCompleted(payload) => {
                turn.status = Some("completed".to_string());
                turn.usage = Some(payload.usage.clone());
            }
            EventPayload::TurnFailed(payload) => {
                turn.status = Some("failed".to_string());
                if !payload.error_message.trim().is_empty() {
                    turn.errors.insert(payload.error_message.trim().to_string());
                    turn.error_event_refs.push(event_ref);
                }
            }
            EventPayload::ItemCompleted(payload) => {
                turn.observe_item_details(&payload.details, &event_ref);
            }
            EventPayload::Error(payload) => {
                if !payload.message.trim().is_empty() {
                    turn.errors.insert(payload.message.trim().to_string());
                    turn.error_event_refs.push(event_ref);
                }
            }
            EventPayload::ThreadStarted(_)
            | EventPayload::ThreadClosed
            | EventPayload::ItemStarted(_)
            | EventPayload::ItemUpdated(_)
            | EventPayload::ItemDelta(_) => {}
        }
    }

    pub fn build(self) -> Option<SessionSummary> {
        let thread_id = self.thread_id?;
        let created_at = self.created_at.unwrap_or_default();
        let updated_at = self.updated_at.unwrap_or(created_at);

        let mut ordered_turn_ids = self.turn_order;
        ordered_turn_ids.sort_by(|left_id, right_id| {
            let left_timestamp = self
                .turns
                .get(left_id)
                .and_then(|turn| turn.started_at)
                .unwrap_or(i64::MAX);
            let right_timestamp = self
                .turns
                .get(right_id)
                .and_then(|turn| turn.started_at)
                .unwrap_or(i64::MAX);
            (left_timestamp, left_id).cmp(&(right_timestamp, right_id))
        });
        ordered_turn_ids.dedup();

        let root_node_id = format!("session:{thread_id}");
        let mut nodes = Vec::new();

        let mut has_failed_turn = false;
        let mut all_turns_completed = !ordered_turn_ids.is_empty();
        for turn_id in &ordered_turn_ids {
            if let Some(turn) = self.turns.get(turn_id) {
                if turn.status.as_deref() == Some("failed") {
                    has_failed_turn = true;
                }
                if turn.status.as_deref() != Some("completed") {
                    all_turns_completed = false;
                }
            }
        }

        let session_status = if has_failed_turn {
            "failed"
        } else if all_turns_completed {
            "completed"
        } else {
            "in_progress"
        };

        nodes.push(SummaryNode {
            node_id: root_node_id.clone(),
            parent_id: None,
            node_type: SummaryNodeType::Session,
            title: format!("Session {thread_id}"),
            summary: Some(format!("{} turn(s)", ordered_turn_ids.len())),
            status: Some(session_status.to_string()),
            turn_id: None,
            promoted: false,
            evidence: SummaryEvidence::default(),
            lineage: None,
        });

        for (index, turn_id) in ordered_turn_ids.iter().enumerate() {
            let Some(turn) = self.turns.get(turn_id) else {
                continue;
            };

            let parent_turn_id = index
                .checked_sub(1)
                .and_then(|previous_index| ordered_turn_ids.get(previous_index).cloned());

            let turn_node_id = format!("turn:{turn_id}");
            let turn_summary = turn.last_agent_message.clone().or_else(|| {
                turn.usage.as_ref().map(|usage| {
                    let total_input_tokens = usage.input_tokens + usage.cached_input_tokens;
                    format!(
                        "tokens in/out: {total_input_tokens}/{}",
                        usage.output_tokens
                    )
                })
            });

            nodes.push(SummaryNode {
                node_id: turn_node_id.clone(),
                parent_id: Some(root_node_id.clone()),
                node_type: SummaryNodeType::Turn,
                title: format!("Turn {turn_id}"),
                summary: turn_summary,
                status: Some(
                    turn.status
                        .clone()
                        .unwrap_or_else(|| "in_progress".to_string()),
                ),
                turn_id: Some(turn_id.clone()),
                promoted: false,
                evidence: SummaryEvidence {
                    event_refs: turn.event_refs.clone(),
                    file_paths: turn.file_paths.iter().cloned().collect(),
                    commands: turn
                        .commands
                        .iter()
                        .map(|command| CommandEvidence {
                            command: command.command.clone(),
                            exit_code: command.exit_code,
                        })
                        .collect(),
                    external_tools: turn.external_tools.iter().cloned().collect(),
                    errors: turn.errors.iter().cloned().collect(),
                },
                lineage: Some(TurnLineage {
                    parent_turn_id,
                    ..Default::default()
                }),
            });

            if !turn.plan_updates.is_empty() {
                nodes.push(SummaryNode {
                    node_id: format!("{turn_node_id}:plan"),
                    parent_id: Some(turn_node_id.clone()),
                    node_type: SummaryNodeType::Plan,
                    title: "Plan".to_string(),
                    summary: Some(turn.plan_updates.join(" | ")),
                    status: Some("info".to_string()),
                    turn_id: Some(turn_id.clone()),
                    promoted: turn.plan_updates.len() >= MAJOR_PLAN_PIVOT_THRESHOLD,
                    evidence: SummaryEvidence {
                        event_refs: turn.plan_event_refs.clone(),
                        ..Default::default()
                    },
                    lineage: None,
                });
            }

            if !turn.commands.is_empty() {
                let execution_status = if turn.has_failed_command {
                    "error"
                } else {
                    "completed"
                };
                nodes.push(SummaryNode {
                    node_id: format!("{turn_node_id}:execution"),
                    parent_id: Some(turn_node_id.clone()),
                    node_type: SummaryNodeType::Execution,
                    title: "Execution".to_string(),
                    summary: Some(format!("{} command(s)", turn.commands.len())),
                    status: Some(execution_status.to_string()),
                    turn_id: Some(turn_id.clone()),
                    promoted: turn.has_failed_command,
                    evidence: SummaryEvidence {
                        event_refs: turn.execution_event_refs.clone(),
                        commands: turn
                            .commands
                            .iter()
                            .map(|command| CommandEvidence {
                                command: command.command.clone(),
                                exit_code: command.exit_code,
                            })
                            .collect(),
                        errors: turn.errors.iter().cloned().collect(),
                        ..Default::default()
                    },
                    lineage: None,
                });
            }

            if !turn.file_paths.is_empty() {
                let code_change_status = if turn.has_failed_patch {
                    "error"
                } else {
                    "completed"
                };
                nodes.push(SummaryNode {
                    node_id: format!("{turn_node_id}:code_changes"),
                    parent_id: Some(turn_node_id.clone()),
                    node_type: SummaryNodeType::CodeChanges,
                    title: "Code Changes".to_string(),
                    summary: Some(format!("{} file(s)", turn.file_paths.len())),
                    status: Some(code_change_status.to_string()),
                    turn_id: Some(turn_id.clone()),
                    promoted: turn.has_failed_patch
                        || turn.file_paths.len() >= LARGE_EDIT_FILE_THRESHOLD,
                    evidence: SummaryEvidence {
                        event_refs: turn.code_change_event_refs.clone(),
                        file_paths: turn.file_paths.iter().cloned().collect(),
                        errors: turn.errors.iter().cloned().collect(),
                        ..Default::default()
                    },
                    lineage: None,
                });
            }

            if !turn.external_tools.is_empty() {
                let tool_status = if turn.has_failed_external_tool {
                    "error"
                } else {
                    "completed"
                };
                nodes.push(SummaryNode {
                    node_id: format!("{turn_node_id}:external_tools"),
                    parent_id: Some(turn_node_id.clone()),
                    node_type: SummaryNodeType::ExternalTools,
                    title: "External Tools".to_string(),
                    summary: Some(format!("{} tool call(s)", turn.external_tools.len())),
                    status: Some(tool_status.to_string()),
                    turn_id: Some(turn_id.clone()),
                    promoted: turn.has_failed_external_tool,
                    evidence: SummaryEvidence {
                        event_refs: turn.external_tool_event_refs.clone(),
                        external_tools: turn.external_tools.iter().cloned().collect(),
                        errors: turn.errors.iter().cloned().collect(),
                        ..Default::default()
                    },
                    lineage: None,
                });
            }

            if !turn.errors.is_empty() {
                nodes.push(SummaryNode {
                    node_id: format!("{turn_node_id}:error"),
                    parent_id: Some(turn_node_id),
                    node_type: SummaryNodeType::Error,
                    title: "Failures".to_string(),
                    summary: Some(format!("{} error(s)", turn.errors.len())),
                    status: Some("error".to_string()),
                    turn_id: Some(turn_id.clone()),
                    promoted: true,
                    evidence: SummaryEvidence {
                        event_refs: turn.error_event_refs.clone(),
                        errors: turn.errors.iter().cloned().collect(),
                        ..Default::default()
                    },
                    lineage: None,
                });
            }
        }

        Some(SessionSummary {
            schema_version: SESSION_SUMMARY_SCHEMA_VERSION.to_string(),
            thread_id,
            root_node_id,
            metadata: SessionSummaryMetadata {
                created_at,
                updated_at,
                turn_count: u32::try_from(ordered_turn_ids.len()).unwrap_or(u32::MAX),
                sources: self.sources.into_sources(),
            },
            nodes,
        })
    }

    pub fn summarize<'a, I>(events: I) -> Option<SessionSummary>
    where
        I: IntoIterator<Item = &'a NormalizedEvent>,
    {
        let mut summarizer = SessionSummarizer::new();
        for event in events {
            summarizer.ingest(event);
        }
        summarizer.build()
    }

    fn turn_mut(&mut self, turn_id: &str, timestamp: i64) -> &mut TurnAccumulator {
        if !self.turns.contains_key(turn_id) {
            self.turn_order.push(turn_id.to_string());
        }

        self.turns
            .entry(turn_id.to_string())
            .or_insert_with(|| TurnAccumulator {
                started_at: Some(timestamp),
                ..Default::default()
            })
    }
}

#[derive(Default)]
struct SourceTracker {
    saw_exec_json: bool,
    saw_app_server_v2: bool,
    saw_rollout_replay: bool,
}

impl SourceTracker {
    fn observe(&mut self, source: &EventSource) {
        match source {
            EventSource::ExecJson => self.saw_exec_json = true,
            EventSource::AppServerV2 => self.saw_app_server_v2 = true,
            EventSource::RolloutReplay => self.saw_rollout_replay = true,
        }
    }

    fn into_sources(self) -> Vec<EventSource> {
        let mut sources = Vec::new();
        if self.saw_exec_json {
            sources.push(EventSource::ExecJson);
        }
        if self.saw_app_server_v2 {
            sources.push(EventSource::AppServerV2);
        }
        if self.saw_rollout_replay {
            sources.push(EventSource::RolloutReplay);
        }
        sources
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CommandEvidenceKey {
    command: String,
    exit_code: Option<i32>,
}

#[derive(Default)]
struct TurnAccumulator {
    started_at: Option<i64>,
    status: Option<String>,
    usage: Option<TokenUsage>,
    last_agent_message: Option<String>,
    plan_updates: Vec<String>,
    commands: BTreeSet<CommandEvidenceKey>,
    file_paths: BTreeSet<String>,
    external_tools: BTreeSet<String>,
    errors: BTreeSet<String>,
    event_refs: Vec<EventReference>,
    plan_event_refs: Vec<EventReference>,
    execution_event_refs: Vec<EventReference>,
    code_change_event_refs: Vec<EventReference>,
    external_tool_event_refs: Vec<EventReference>,
    error_event_refs: Vec<EventReference>,
    has_failed_command: bool,
    has_failed_patch: bool,
    has_failed_external_tool: bool,
}

impl TurnAccumulator {
    fn observe_item_details(&mut self, details: &ItemDetails, event_ref: &EventReference) {
        match details {
            ItemDetails::AgentMessage(details) => {
                let message = details.text.trim();
                if !message.is_empty() {
                    self.last_agent_message = Some(message.to_string());
                }
            }
            ItemDetails::Reasoning(_details) => {}
            ItemDetails::CommandExecution(details) => {
                let command = details.command.trim();
                if !command.is_empty() {
                    self.commands.insert(CommandEvidenceKey {
                        command: command.to_string(),
                        exit_code: details.exit_code,
                    });
                }
                self.execution_event_refs.push(event_ref.clone());

                let command_failed = details.status != ExecutionStatus::Completed
                    || details.exit_code.is_some_and(|exit_code| exit_code != 0);
                if command_failed {
                    self.has_failed_command = true;
                    let output = details.aggregated_output.trim();
                    if !output.is_empty() {
                        self.errors.insert(output.to_string());
                    } else if !command.is_empty() {
                        self.errors.insert(format!("command failed: {command}"));
                    }
                    self.error_event_refs.push(event_ref.clone());
                }
            }
            ItemDetails::FileChange(details) => {
                self.code_change_event_refs.push(event_ref.clone());
                for change in &details.changes {
                    let path = change.path.trim();
                    if !path.is_empty() {
                        self.file_paths.insert(path.to_string());
                    }
                }
                if details.status != PatchStatus::Completed {
                    self.has_failed_patch = true;
                    self.errors.insert("patch apply failed".to_string());
                    self.error_event_refs.push(event_ref.clone());
                }
            }
            ItemDetails::McpToolCall(details) => {
                self.external_tool_event_refs.push(event_ref.clone());
                self.external_tools
                    .insert(format!("{}/{}", details.server, details.tool));

                let failed = details.status != McpStatus::Completed || details.error.is_some();
                if failed {
                    self.has_failed_external_tool = true;
                    if let Some(error) = &details.error
                        && !error.message.trim().is_empty()
                    {
                        self.errors.insert(error.message.trim().to_string());
                    }
                    if details.error.is_none() {
                        self.errors.insert(format!(
                            "tool call failed: {}/{}",
                            details.server, details.tool
                        ));
                    }
                    self.error_event_refs.push(event_ref.clone());
                }
            }
            ItemDetails::CollabToolCall(details) => {
                self.external_tool_event_refs.push(event_ref.clone());
                self.external_tools
                    .insert(format!("collab::{:?}", details.tool));

                let any_agent_failed = details.agents_states.values().any(|agent_state| {
                    matches!(
                        agent_state.status,
                        CollabAgentStatus::Errored | CollabAgentStatus::NotFound
                    )
                });
                if details.status != CollabStatus::Completed || any_agent_failed {
                    self.has_failed_external_tool = true;
                    self.errors
                        .insert(format!("collab tool failed: {:?}", details.tool));
                    self.error_event_refs.push(event_ref.clone());
                }
            }
            ItemDetails::WebSearch(details) => {
                self.external_tool_event_refs.push(event_ref.clone());
                let query = details.query.trim();
                if !query.is_empty() {
                    self.external_tools.insert(format!("web_search: {query}"));
                }
            }
            ItemDetails::TodoList(details) => {
                self.plan_event_refs.push(event_ref.clone());
                let plan_update = details
                    .items
                    .iter()
                    .map(|item| {
                        if item.completed {
                            format!("[x] {}", item.text)
                        } else {
                            format!("[ ] {}", item.text)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                if !plan_update.is_empty() && !self.plan_updates.contains(&plan_update) {
                    self.plan_updates.push(plan_update);
                }
            }
            ItemDetails::Error(details) => {
                let message = details.message.trim();
                if !message.is_empty() {
                    self.errors.insert(message.to_string());
                    self.error_event_refs.push(event_ref.clone());
                }
            }
        }
    }
}
