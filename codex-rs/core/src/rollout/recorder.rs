//! Persist Codex session rollouts (.jsonl) so sessions can be replayed or inspected later.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs::File;
use std::fs::{self};
use std::io::Error as IoError;
use std::path::Path;
use std::path::PathBuf;

use chrono::SecondsFormat;
use codex_protocol::ThreadId;
use codex_protocol::dynamic_tools::DynamicToolSpec;
use codex_protocol::models::BaseInstructions;
use codex_protocol::models::LocalShellAction;
use codex_protocol::models::ResponseItem;
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::FormatItem;
use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc::Sender;
use tokio::sync::mpsc::{self};
use tokio::sync::oneshot;
use tracing::info;
use tracing::trace;
use tracing::warn;

use super::ARCHIVED_SESSIONS_SUBDIR;
use super::SESSIONS_SUBDIR;
use super::list::Cursor;
use super::list::ThreadItem;
use super::list::ThreadListConfig;
use super::list::ThreadListLayout;
use super::list::ThreadSortKey;
use super::list::ThreadsPage;
use super::list::get_threads;
use super::list::get_threads_in_root;
use super::list::parse_cursor;
use super::list::parse_timestamp_uuid_from_filename;
use super::metadata;
use super::policy::EventPersistenceMode;
use super::policy::is_persisted_response_item;
use super::turn_summary_llm::TurnSummaryEvidence;
use super::turn_summary_llm::generate_turn_summaries;
use crate::config::Config;
use crate::default_client::originator;
use crate::git_info::collect_git_info;
use crate::path_utils;
use crate::state_db;
use crate::state_db::StateDbHandle;
use crate::truncate::TruncationPolicy;
use crate::truncate::truncate_text;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::InitialHistory;
use codex_protocol::protocol::ResumedHistory;
use codex_protocol::protocol::RolloutItem;
use codex_protocol::protocol::RolloutLine;
use codex_protocol::protocol::SessionMeta;
use codex_protocol::protocol::SessionMetaLine;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::TurnAbortReason;
use codex_state::SessionSummaryPersistParams;
use codex_state::StateRuntime;
use codex_state::ThreadMetadataBuilder;

/// Records all [`ResponseItem`]s for a session and flushes them to disk after
/// every update.
///
/// Rollouts are recorded as JSONL and can be inspected with tools such as:
///
/// ```ignore
/// $ jq -C . ~/.codex/sessions/rollout-2025-05-07T17-24-21-5973b6c0-94b8-487b-a530-2aeb6098ae0e.jsonl
/// $ fx ~/.codex/sessions/rollout-2025-05-07T17-24-21-5973b6c0-94b8-487b-a530-2aeb6098ae0e.jsonl
/// ```
#[derive(Clone)]
pub struct RolloutRecorder {
    tx: Sender<RolloutCmd>,
    pub(crate) rollout_path: PathBuf,
    state_db: Option<StateDbHandle>,
    event_persistence_mode: EventPersistenceMode,
}

#[derive(Clone)]
pub enum RolloutRecorderParams {
    Create {
        conversation_id: ThreadId,
        forked_from_id: Option<ThreadId>,
        source: SessionSource,
        base_instructions: BaseInstructions,
        dynamic_tools: Vec<DynamicToolSpec>,
        event_persistence_mode: EventPersistenceMode,
    },
    Resume {
        path: PathBuf,
        event_persistence_mode: EventPersistenceMode,
    },
}

enum RolloutCmd {
    AddItems(Vec<RolloutItem>),
    Persist {
        ack: oneshot::Sender<()>,
    },
    /// Ensure all prior writes are processed; respond when flushed.
    Flush {
        ack: oneshot::Sender<()>,
    },
    Shutdown {
        ack: oneshot::Sender<()>,
    },
}

impl RolloutRecorderParams {
    pub fn new(
        conversation_id: ThreadId,
        forked_from_id: Option<ThreadId>,
        source: SessionSource,
        base_instructions: BaseInstructions,
        dynamic_tools: Vec<DynamicToolSpec>,
        event_persistence_mode: EventPersistenceMode,
    ) -> Self {
        Self::Create {
            conversation_id,
            forked_from_id,
            source,
            base_instructions,
            dynamic_tools,
            event_persistence_mode,
        }
    }

    pub fn resume(path: PathBuf, event_persistence_mode: EventPersistenceMode) -> Self {
        Self::Resume {
            path,
            event_persistence_mode,
        }
    }
}

const PERSISTED_EXEC_AGGREGATED_OUTPUT_MAX_BYTES: usize = 10_000;
const AGENTCANVAS_TURN_SUMMARY_SCHEMA_VERSION: &str = "agentcanvas.turn.v2";
const AGENTCANVAS_TURN_SUMMARY_KIND: &str = "agentcanvas_turn_summary";
const SUMMARY_MAX_AGENT_MESSAGE_BYTES: usize = 480;
const SUMMARY_MAX_COMMAND_TEXT_BYTES: usize = 320;
const SUMMARY_MAX_FILE_PATH_TEXT_BYTES: usize = 280;
const SUMMARY_MAX_ERROR_TEXT_BYTES: usize = 420;
const SUMMARY_MAX_COMMAND_ITEMS: usize = 30;
const SUMMARY_MAX_FILE_PATH_ITEMS: usize = 40;
const SUMMARY_MAX_ERROR_ITEMS: usize = 20;
const SUMMARY_DIGEST_ITEMS: usize = 5;

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, serde::Serialize)]
struct AgentCanvasCommandEvidence {
    command: String,
    exit_code: Option<i64>,
}

#[derive(Debug, Clone)]
struct PendingAgentCanvasTurnSummary {
    turn_id: String,
    parent_turn_id: Option<String>,
    started_after_rollback: bool,
    commands: BTreeSet<AgentCanvasCommandEvidence>,
    file_paths: BTreeSet<String>,
    errors: BTreeSet<String>,
}

impl PendingAgentCanvasTurnSummary {
    fn new(turn_id: String, parent_turn_id: Option<String>, started_after_rollback: bool) -> Self {
        Self {
            turn_id,
            parent_turn_id,
            started_after_rollback,
            commands: BTreeSet::new(),
            file_paths: BTreeSet::new(),
            errors: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone)]
struct CompletedAgentCanvasTurnSummary {
    turn_id: String,
    status: String,
    parent_turn_id: Option<String>,
    forked_from_thread_id: Option<String>,
    started_after_rollback: bool,
    last_agent_message: Option<String>,
    commands: Vec<AgentCanvasCommandEvidence>,
    file_paths: Vec<String>,
    errors: Vec<String>,
}

#[derive(Default)]
struct AgentCanvasSummaryUpdates {
    completed_turn_summaries: Vec<CompletedAgentCanvasTurnSummary>,
    rolled_back_turn_ids: Vec<String>,
}

impl AgentCanvasSummaryUpdates {
    fn append(&mut self, mut other: Self) {
        self.completed_turn_summaries
            .append(&mut other.completed_turn_summaries);
        self.rolled_back_turn_ids
            .append(&mut other.rolled_back_turn_ids);
    }
}

#[derive(Default)]
struct AgentCanvasSummaryAccumulator {
    thread_id: Option<String>,
    forked_from_thread_id: Option<String>,
    pending_turn: Option<PendingAgentCanvasTurnSummary>,
    active_turn_lineage: Vec<String>,
    rollback_since_last_turn: bool,
}

impl AgentCanvasSummaryAccumulator {
    fn update_thread_id(&mut self, builder: Option<&ThreadMetadataBuilder>, rollout_path: &Path) {
        if self.thread_id.is_none()
            && let Some(builder) = builder
        {
            self.thread_id = Some(builder.id.to_string());
        }
        if self.thread_id.is_none()
            && let Some(file_name) = rollout_path.file_name().and_then(|name| name.to_str())
            && let Some((_timestamp, id)) = parse_timestamp_uuid_from_filename(file_name)
        {
            self.thread_id = Some(id.to_string());
        }
    }

    fn observe_session_meta(&mut self, meta_line: &SessionMetaLine) {
        if self.thread_id.is_none() {
            self.thread_id = Some(meta_line.meta.id.to_string());
        }
        if self.forked_from_thread_id.is_none()
            && let Some(forked_from_id) = meta_line.meta.forked_from_id.as_ref()
        {
            self.forked_from_thread_id = Some(forked_from_id.to_string());
        }
    }

    fn handle_rollout_items(&mut self, items: &[RolloutItem]) -> AgentCanvasSummaryUpdates {
        let mut updates = AgentCanvasSummaryUpdates::default();
        for item in items {
            updates.append(self.handle_rollout_item(item));
        }
        updates
    }

    fn handle_rollout_item(&mut self, item: &RolloutItem) -> AgentCanvasSummaryUpdates {
        match item {
            RolloutItem::EventMsg(event) => self.handle_event(event),
            RolloutItem::ResponseItem(item) => {
                self.handle_response_item(item);
                AgentCanvasSummaryUpdates::default()
            }
            RolloutItem::SessionMeta(meta_line) => {
                self.observe_session_meta(meta_line);
                AgentCanvasSummaryUpdates::default()
            }
            RolloutItem::Compacted(_) | RolloutItem::TurnContext(_) => {
                AgentCanvasSummaryUpdates::default()
            }
        }
    }

    fn handle_event(
        &mut self,
        event: &codex_protocol::protocol::EventMsg,
    ) -> AgentCanvasSummaryUpdates {
        let mut updates = AgentCanvasSummaryUpdates::default();
        match event {
            codex_protocol::protocol::EventMsg::TurnStarted(event) => {
                let parent_turn_id = self.active_turn_lineage.last().cloned();
                self.pending_turn = Some(PendingAgentCanvasTurnSummary::new(
                    event.turn_id.clone(),
                    parent_turn_id,
                    self.rollback_since_last_turn,
                ));
                self.rollback_since_last_turn = false;
            }
            codex_protocol::protocol::EventMsg::TurnComplete(event) => {
                let completed = self.complete_turn_summary(
                    event.turn_id.as_str(),
                    "completed",
                    event.last_agent_message.clone(),
                );
                self.push_active_turn(completed.turn_id.clone());
                updates.completed_turn_summaries.push(completed);
            }
            codex_protocol::protocol::EventMsg::TurnAborted(event) => {
                let Some(turn_id) = event.turn_id.clone().or_else(|| {
                    self.pending_turn
                        .as_ref()
                        .map(|pending| pending.turn_id.clone())
                }) else {
                    return updates;
                };
                let status = match event.reason {
                    TurnAbortReason::Interrupted => "interrupted",
                    TurnAbortReason::Replaced => "replaced",
                    TurnAbortReason::ReviewEnded => "review_ended",
                };
                updates
                    .completed_turn_summaries
                    .push(self.complete_turn_summary(turn_id.as_str(), status, None));
            }
            codex_protocol::protocol::EventMsg::ThreadRolledBack(event) => {
                updates.rolled_back_turn_ids = self.rollback_active_turns(event.num_turns);
                if !updates.rolled_back_turn_ids.is_empty() {
                    self.rollback_since_last_turn = true;
                }
            }
            codex_protocol::protocol::EventMsg::ExecCommandEnd(event) => {
                let turn_id = if event.turn_id.is_empty() {
                    None
                } else {
                    Some(event.turn_id.as_str())
                };
                if let Some(pending) = self.pending_turn_for(turn_id) {
                    pending.commands.insert(AgentCanvasCommandEvidence {
                        command: event.command.join(" "),
                        exit_code: Some(i64::from(event.exit_code)),
                    });
                    if event.exit_code != 0
                        || !matches!(
                            event.status,
                            codex_protocol::protocol::ExecCommandStatus::Completed
                        )
                    {
                        if !event.stderr.trim().is_empty() {
                            pending.errors.insert(event.stderr.trim().to_string());
                        } else if !event.aggregated_output.trim().is_empty() {
                            pending
                                .errors
                                .insert(event.aggregated_output.trim().to_string());
                        }
                    }
                }
            }
            codex_protocol::protocol::EventMsg::PatchApplyEnd(event) => {
                let turn_id = if event.turn_id.is_empty() {
                    None
                } else {
                    Some(event.turn_id.as_str())
                };
                if let Some(pending) = self.pending_turn_for(turn_id) {
                    for path in event.changes.keys() {
                        pending
                            .file_paths
                            .insert(path.to_string_lossy().into_owned());
                    }
                    if !event.success {
                        if !event.stderr.trim().is_empty() {
                            pending.errors.insert(event.stderr.trim().to_string());
                        } else if !event.stdout.trim().is_empty() {
                            pending.errors.insert(event.stdout.trim().to_string());
                        }
                    }
                }
            }
            codex_protocol::protocol::EventMsg::Error(event) => {
                if let Some(pending) = self.pending_turn_for(None) {
                    pending.errors.insert(event.message.clone());
                }
            }
            codex_protocol::protocol::EventMsg::ViewImageToolCall(event) => {
                if let Some(pending) = self.pending_turn_for(None) {
                    pending
                        .file_paths
                        .insert(event.path.to_string_lossy().into_owned());
                }
            }
            _ => {}
        }
        updates
    }

    fn handle_response_item(&mut self, item: &ResponseItem) {
        let Some(pending) = self.pending_turn_for(None) else {
            return;
        };
        match item {
            ResponseItem::LocalShellCall { action, .. } => {
                let LocalShellAction::Exec(exec) = action;
                pending.commands.insert(AgentCanvasCommandEvidence {
                    command: exec.command.join(" "),
                    exit_code: None,
                });
            }
            ResponseItem::FunctionCall {
                name, arguments, ..
            } => {
                collect_apply_patch_paths(
                    name.as_str(),
                    arguments.as_str(),
                    &mut pending.file_paths,
                );
                if let Some(command) = extract_command_from_json(arguments.as_str()) {
                    pending.commands.insert(AgentCanvasCommandEvidence {
                        command,
                        exit_code: None,
                    });
                }
            }
            ResponseItem::FunctionCallOutput { output, .. }
            | ResponseItem::CustomToolCallOutput { output, .. } => {
                if output.success == Some(false)
                    && let Some(text) = output.body.to_text()
                {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        pending.errors.insert(trimmed.to_string());
                    }
                }
            }
            ResponseItem::CustomToolCall { name, input, .. } => {
                collect_apply_patch_paths(name.as_str(), input.as_str(), &mut pending.file_paths);
                if let Some(command) = extract_command_from_json(input.as_str()) {
                    pending.commands.insert(AgentCanvasCommandEvidence {
                        command,
                        exit_code: None,
                    });
                }
            }
            ResponseItem::Message { .. }
            | ResponseItem::Reasoning { .. }
            | ResponseItem::WebSearchCall { .. }
            | ResponseItem::GhostSnapshot { .. }
            | ResponseItem::Compaction { .. }
            | ResponseItem::Other => {}
        }
    }

    fn pending_turn_for(
        &mut self,
        turn_id: Option<&str>,
    ) -> Option<&mut PendingAgentCanvasTurnSummary> {
        match (self.pending_turn.as_ref(), turn_id) {
            (Some(pending), Some(turn_id)) if pending.turn_id != turn_id => None,
            _ => self.pending_turn.as_mut(),
        }
    }

    fn complete_turn_summary(
        &mut self,
        turn_id: &str,
        status: &str,
        last_agent_message: Option<String>,
    ) -> CompletedAgentCanvasTurnSummary {
        let pending_turn = match self.pending_turn.take() {
            Some(pending_turn) if pending_turn.turn_id == turn_id => pending_turn,
            Some(pending_turn) => {
                self.pending_turn = Some(pending_turn);
                PendingAgentCanvasTurnSummary::new(
                    turn_id.to_string(),
                    self.active_turn_lineage.last().cloned(),
                    false,
                )
            }
            None => PendingAgentCanvasTurnSummary::new(
                turn_id.to_string(),
                self.active_turn_lineage.last().cloned(),
                false,
            ),
        };
        CompletedAgentCanvasTurnSummary {
            turn_id: turn_id.to_string(),
            status: status.to_string(),
            parent_turn_id: pending_turn.parent_turn_id,
            forked_from_thread_id: self.forked_from_thread_id.clone(),
            started_after_rollback: pending_turn.started_after_rollback,
            last_agent_message,
            commands: pending_turn.commands.into_iter().collect(),
            file_paths: pending_turn.file_paths.into_iter().collect(),
            errors: pending_turn.errors.into_iter().collect(),
        }
    }

    fn push_active_turn(&mut self, turn_id: String) {
        self.active_turn_lineage.retain(|id| id != turn_id.as_str());
        self.active_turn_lineage.push(turn_id);
    }

    fn rollback_active_turns(&mut self, num_turns: u32) -> Vec<String> {
        let mut rolled_back_turn_ids = Vec::new();
        for _ in 0..num_turns {
            let Some(turn_id) = self.active_turn_lineage.pop() else {
                break;
            };
            rolled_back_turn_ids.push(turn_id);
        }
        rolled_back_turn_ids
    }
}

fn collect_apply_patch_paths(name: &str, input: &str, file_paths: &mut BTreeSet<String>) {
    if name != "apply_patch" {
        return;
    }
    for line in input.lines() {
        for prefix in [
            "*** Add File: ",
            "*** Update File: ",
            "*** Delete File: ",
            "*** Move to: ",
        ] {
            if let Some(path) = line.strip_prefix(prefix) {
                let path = path.trim();
                if !path.is_empty() {
                    file_paths.insert(path.to_string());
                }
            }
        }
    }
}

fn extract_command_from_json(input: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(input).ok()?;
    if let Some(command) = value.get("command").and_then(Value::as_str) {
        return Some(command.to_string());
    }
    if let Some(command) = value.get("cmd").and_then(Value::as_str) {
        return Some(command.to_string());
    }
    None
}

fn normalize_summary_text(value: &str, max_bytes: usize) -> Option<String> {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = normalized.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(truncate_text(trimmed, TruncationPolicy::Bytes(max_bytes)))
}

fn classify_turn_signal(
    total_command_count: usize,
    total_file_path_count: usize,
    total_error_count: usize,
) -> &'static str {
    if total_error_count > 0 {
        return "error";
    }
    if total_file_path_count > 0 {
        return "code_changes";
    }
    if total_command_count > 0 {
        return "execution";
    }
    "status_only"
}

fn build_compact_turn_summary(
    status: &str,
    signal: &str,
    parent_turn_id: Option<&str>,
    child_turn_id: Option<&str>,
    agent_message: Option<&str>,
    command_digest: &[String],
    file_path_digest: &[String],
    error_digest: &[String],
    total_command_count: usize,
    total_file_path_count: usize,
    total_error_count: usize,
) -> String {
    let mut parts = vec![format!("status={status}"), format!("signal={signal}")];
    if total_command_count > 0 {
        parts.push(format!("commands_total={total_command_count}"));
    }
    if total_file_path_count > 0 {
        parts.push(format!("file_paths_total={total_file_path_count}"));
    }
    if total_error_count > 0 {
        parts.push(format!("errors_total={total_error_count}"));
    }
    if let Some(parent_turn_id) = parent_turn_id {
        parts.push(format!("parent_turn={parent_turn_id}"));
    }
    if let Some(child_turn_id) = child_turn_id {
        parts.push(format!("child_turn={child_turn_id}"));
    }
    if let Some(command) = command_digest.first() {
        parts.push(format!("primary_command={command}"));
    }
    if let Some(file_path) = file_path_digest.first() {
        parts.push(format!("primary_file_path={file_path}"));
    }
    if let Some(error) = error_digest.first() {
        parts.push(format!("primary_error={error}"));
    }
    if let Some(agent_message) = agent_message {
        parts.push(format!("agent_message={agent_message}"));
    }
    truncate_text(
        parts.join("; ").as_str(),
        TruncationPolicy::Bytes(SUMMARY_MAX_AGENT_MESSAGE_BYTES),
    )
}

fn compact_command_evidence(
    commands: Vec<AgentCanvasCommandEvidence>,
) -> (Vec<AgentCanvasCommandEvidence>, usize, usize) {
    let mut normalized_commands = BTreeSet::new();
    for command in commands {
        let Some(command_text) =
            normalize_summary_text(command.command.as_str(), SUMMARY_MAX_COMMAND_TEXT_BYTES)
        else {
            continue;
        };
        normalized_commands.insert(AgentCanvasCommandEvidence {
            command: command_text,
            exit_code: command.exit_code,
        });
    }
    let total_commands = normalized_commands.len();
    let kept_commands: Vec<AgentCanvasCommandEvidence> = normalized_commands
        .into_iter()
        .take(SUMMARY_MAX_COMMAND_ITEMS)
        .collect();
    let omitted_commands = total_commands.saturating_sub(kept_commands.len());
    (kept_commands, total_commands, omitted_commands)
}

fn compact_string_evidence(
    values: Vec<String>,
    max_items: usize,
    max_text_bytes: usize,
) -> (Vec<String>, usize, usize) {
    let mut normalized_values = BTreeSet::new();
    for value in values {
        if let Some(normalized) = normalize_summary_text(value.as_str(), max_text_bytes) {
            normalized_values.insert(normalized);
        }
    }
    let total_values = normalized_values.len();
    let kept_values: Vec<String> = normalized_values.into_iter().take(max_items).collect();
    let omitted_values = total_values.saturating_sub(kept_values.len());
    (kept_values, total_values, omitted_values)
}

fn build_digest_strings(values: &[String]) -> Vec<String> {
    values.iter().take(SUMMARY_DIGEST_ITEMS).cloned().collect()
}

fn build_digest_commands(commands: &[AgentCanvasCommandEvidence]) -> Vec<String> {
    commands
        .iter()
        .take(SUMMARY_DIGEST_ITEMS)
        .map(|command| command.command.clone())
        .collect()
}

async fn persist_agentcanvas_turn_summaries(
    state_db_ctx: Option<&StateRuntime>,
    accumulator: &AgentCanvasSummaryAccumulator,
    summary_updates: AgentCanvasSummaryUpdates,
) {
    let (Some(state_db_ctx), Some(thread_id)) = (state_db_ctx, accumulator.thread_id.as_deref())
    else {
        return;
    };

    let completed_turn_summaries = summary_updates.completed_turn_summaries;
    let completed_turn_count = completed_turn_summaries.len();
    struct PersistedTurnSummary {
        turn_id: String,
        status: String,
        parent_turn_id: Option<String>,
        child_turn_id: Option<String>,
        forked_from_thread_id: Option<String>,
        started_after_rollback: bool,
        node_id: String,
        parent_node_id: Option<String>,
        signal: String,
        agent_message: Option<String>,
        command_digest: Vec<String>,
        file_path_digest: Vec<String>,
        error_digest: Vec<String>,
        commands: Vec<AgentCanvasCommandEvidence>,
        file_paths: Vec<String>,
        errors: Vec<String>,
        total_command_count: usize,
        total_file_path_count: usize,
        total_error_count: usize,
        omitted_command_count: usize,
        omitted_file_path_count: usize,
        omitted_error_count: usize,
        compact_summary: String,
    }

    let mut persisted_turn_summaries = Vec::with_capacity(completed_turn_count);
    let mut llm_inputs = Vec::with_capacity(completed_turn_count);

    for (index, turn_summary) in completed_turn_summaries.iter().enumerate() {
        let turn_summary = turn_summary.clone();
        let CompletedAgentCanvasTurnSummary {
            turn_id,
            status,
            parent_turn_id,
            forked_from_thread_id,
            started_after_rollback,
            last_agent_message,
            commands,
            file_paths,
            errors,
        } = turn_summary;
        let node_id = format!("turn:{turn_id}");
        let parent_node_id = parent_turn_id
            .as_ref()
            .map(|parent_turn_id| format!("turn:{parent_turn_id}"));
        let child_turn_id = (index + 1 < completed_turn_count)
            .then(|| completed_turn_summaries[index + 1].turn_id.as_str());
        let (commands, total_command_count, omitted_command_count) =
            compact_command_evidence(commands);
        let (file_paths, total_file_path_count, omitted_file_path_count) = compact_string_evidence(
            file_paths,
            SUMMARY_MAX_FILE_PATH_ITEMS,
            SUMMARY_MAX_FILE_PATH_TEXT_BYTES,
        );
        let (errors, total_error_count, omitted_error_count) = compact_string_evidence(
            errors,
            SUMMARY_MAX_ERROR_ITEMS,
            SUMMARY_MAX_ERROR_TEXT_BYTES,
        );
        let agent_message = last_agent_message
            .as_deref()
            .and_then(|text| normalize_summary_text(text, SUMMARY_MAX_AGENT_MESSAGE_BYTES));
        let command_digest = build_digest_commands(commands.as_slice());
        let file_path_digest = build_digest_strings(file_paths.as_slice());
        let error_digest = build_digest_strings(errors.as_slice());
        let signal = classify_turn_signal(
            total_command_count,
            total_file_path_count,
            total_error_count,
        );
        let summary_text = build_compact_turn_summary(
            status.as_str(),
            signal,
            parent_turn_id.as_deref(),
            child_turn_id,
            agent_message.as_deref(),
            command_digest.as_slice(),
            file_path_digest.as_slice(),
            error_digest.as_slice(),
            total_command_count,
            total_file_path_count,
            total_error_count,
        );
        let child_turn_id = child_turn_id.map(ToOwned::to_owned);
        let command_digest_for_evidence = command_digest.first().cloned();
        let file_path_digest_for_evidence = file_path_digest.first().cloned();
        let error_digest_for_evidence = error_digest.first().cloned();
        llm_inputs.push(TurnSummaryEvidence {
            turn_id: turn_id.clone(),
            status: status.clone(),
            parent_turn_id: parent_turn_id.clone(),
            child_turn_id: child_turn_id.clone(),
            forked_from_thread_id: forked_from_thread_id.clone(),
            started_after_rollback,
            signal: signal.to_string(),
            last_agent_message: agent_message.clone(),
            primary_command: command_digest_for_evidence,
            primary_file_path: file_path_digest_for_evidence,
            primary_error: error_digest_for_evidence,
            total_commands: total_command_count,
            total_file_paths: total_file_path_count,
            total_errors: total_error_count,
        });

        persisted_turn_summaries.push(PersistedTurnSummary {
            turn_id,
            status,
            parent_turn_id,
            child_turn_id,
            forked_from_thread_id,
            started_after_rollback,
            node_id,
            parent_node_id,
            signal: signal.to_string(),
            agent_message: agent_message.clone(),
            command_digest,
            file_path_digest,
            error_digest,
            commands,
            file_paths,
            errors,
            total_command_count,
            total_file_path_count,
            total_error_count,
            omitted_command_count,
            omitted_file_path_count,
            omitted_error_count,
            compact_summary: summary_text,
        });
    }

    let llm_summaries = match generate_turn_summaries(&llm_inputs).await {
        Ok(summary_map) => summary_map,
        Err(err) => {
            if !llm_inputs.is_empty() {
                warn!("failed to generate LLM turn summaries: {err}");
            }
            BTreeMap::new()
        }
    };

    for persisted_turn_summary in persisted_turn_summaries {
        let PersistedTurnSummary {
            turn_id,
            status,
            parent_turn_id,
            child_turn_id,
            forked_from_thread_id,
            started_after_rollback,
            node_id,
            parent_node_id,
            signal,
            agent_message,
            command_digest,
            file_path_digest,
            error_digest,
            commands,
            file_paths,
            errors,
            total_command_count,
            total_file_path_count,
            total_error_count,
            omitted_command_count,
            omitted_file_path_count,
            omitted_error_count,
            compact_summary,
        } = persisted_turn_summary;
        let summary = llm_summaries
            .get(turn_id.as_str())
            .cloned()
            .unwrap_or(compact_summary);
        let summary = serde_json::json!({
            "schema_version": AGENTCANVAS_TURN_SUMMARY_SCHEMA_VERSION,
            "summary_kind": AGENTCANVAS_TURN_SUMMARY_KIND,
            "thread_id": thread_id,
            "session_id": turn_id.clone(),
            "forked_from_thread_id": forked_from_thread_id.clone(),
            "nodes": [
                {
                    "node_id": node_id,
                    "parent_id": parent_node_id,
                    "node_type": "turn",
                    "title": format!("Turn {turn_id}"),
                    "summary": summary,
                    "status": status,
                    "brief": {
                        "signal": signal,
                        "agent_message": agent_message,
                        "primary_command": command_digest.first().cloned(),
                        "primary_file_path": file_path_digest.first().cloned(),
                        "primary_error": error_digest.first().cloned(),
                    },
                    "lineage": {
                        "parent_turn_id": parent_turn_id,
                        "child_turn_id": child_turn_id,
                        "forked_from_thread_id": forked_from_thread_id,
                        "started_after_rollback": started_after_rollback,
                    },
                    "counts": {
                        "commands_total": total_command_count,
                        "commands_indexed": commands.len(),
                        "commands_omitted": omitted_command_count,
                        "file_paths_total": total_file_path_count,
                        "file_paths_indexed": file_paths.len(),
                        "file_paths_omitted": omitted_file_path_count,
                        "errors_total": total_error_count,
                        "errors_indexed": errors.len(),
                        "errors_omitted": omitted_error_count,
                    },
                    "digest": {
                        "command_examples": command_digest,
                        "file_path_examples": file_path_digest,
                        "error_examples": error_digest,
                    },
                    "evidence": {
                        "file_paths": file_paths,
                        "commands": commands,
                        "errors": errors,
                    }
                }
            ]
        });
        let params = SessionSummaryPersistParams {
            summary_id: Some(format!("agentcanvas.turn:{thread_id}:{turn_id}")),
            thread_id: thread_id.to_string(),
            session_id: turn_id.clone(),
            schema_version: AGENTCANVAS_TURN_SUMMARY_SCHEMA_VERSION.to_string(),
            root_node_id: Some(node_id),
            summary,
        };
        if let Err(err) = state_db_ctx.upsert_session_summary(&params).await {
            warn!(
                "failed to persist agentcanvas turn summary for thread {} turn {}: {err}",
                thread_id, turn_id
            );
        }
    }

    for turn_id in summary_updates.rolled_back_turn_ids {
        let Ok(Some(mut summary)) = state_db_ctx
            .read_session_summary_by_thread_and_session(thread_id, turn_id.as_str())
            .await
        else {
            continue;
        };
        let node_id = format!("turn:{turn_id}");
        let mut updated = false;
        if let Some(nodes) = summary.get_mut("nodes").and_then(Value::as_array_mut) {
            for node in nodes {
                let matches_node = node
                    .get("node_id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id == node_id);
                if !matches_node {
                    continue;
                }
                let Some(node_object) = node.as_object_mut() else {
                    continue;
                };
                node_object.insert(
                    "status".to_string(),
                    Value::String("rolled_back".to_string()),
                );
                if !matches!(node_object.get("lineage"), Some(Value::Object(_))) {
                    node_object.insert("lineage".to_string(), serde_json::json!({}));
                }
                if let Some(lineage) = node_object
                    .get_mut("lineage")
                    .and_then(Value::as_object_mut)
                {
                    lineage.insert("was_rolled_back".to_string(), Value::Bool(true));
                }
                updated = true;
            }
        }
        if !updated {
            continue;
        }

        let root_node_id = summary
            .get("nodes")
            .and_then(Value::as_array)
            .and_then(|nodes| nodes.first())
            .and_then(|node| node.get("node_id"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let params = SessionSummaryPersistParams {
            summary_id: Some(format!("agentcanvas.turn:{thread_id}:{turn_id}")),
            thread_id: thread_id.to_string(),
            session_id: turn_id.clone(),
            schema_version: summary
                .get("schema_version")
                .and_then(Value::as_str)
                .unwrap_or(AGENTCANVAS_TURN_SUMMARY_SCHEMA_VERSION)
                .to_string(),
            root_node_id,
            summary,
        };
        if let Err(err) = state_db_ctx.upsert_session_summary(&params).await {
            warn!(
                "failed to mark agentcanvas turn summary rolled back for thread {} turn {}: {err}",
                thread_id, turn_id
            );
        }
    }
}

fn sanitize_rollout_item_for_persistence(
    item: RolloutItem,
    mode: EventPersistenceMode,
) -> RolloutItem {
    if mode != EventPersistenceMode::Extended {
        return item;
    }

    match item {
        RolloutItem::EventMsg(EventMsg::ExecCommandEnd(mut event)) => {
            // Persist only a bounded aggregated summary of command output.
            event.aggregated_output = truncate_text(
                &event.aggregated_output,
                TruncationPolicy::Bytes(PERSISTED_EXEC_AGGREGATED_OUTPUT_MAX_BYTES),
            );
            // Drop unnecessary fields from rollout storage since aggregated_output is all we need.
            event.stdout.clear();
            event.stderr.clear();
            event.formatted_output.clear();
            RolloutItem::EventMsg(EventMsg::ExecCommandEnd(event))
        }
        _ => item,
    }
}

impl RolloutRecorder {
    /// List threads (rollout files) under the provided Codex home directory.
    #[allow(clippy::too_many_arguments)]
    pub async fn list_threads(
        config: &Config,
        page_size: usize,
        cursor: Option<&Cursor>,
        sort_key: ThreadSortKey,
        allowed_sources: &[SessionSource],
        model_providers: Option<&[String]>,
        default_provider: &str,
        search_term: Option<&str>,
    ) -> std::io::Result<ThreadsPage> {
        Self::list_threads_with_db_fallback(
            config,
            page_size,
            cursor,
            sort_key,
            allowed_sources,
            model_providers,
            default_provider,
            false,
            search_term,
        )
        .await
    }

    /// List archived threads (rollout files) under the archived sessions directory.
    #[allow(clippy::too_many_arguments)]
    pub async fn list_archived_threads(
        config: &Config,
        page_size: usize,
        cursor: Option<&Cursor>,
        sort_key: ThreadSortKey,
        allowed_sources: &[SessionSource],
        model_providers: Option<&[String]>,
        default_provider: &str,
        search_term: Option<&str>,
    ) -> std::io::Result<ThreadsPage> {
        Self::list_threads_with_db_fallback(
            config,
            page_size,
            cursor,
            sort_key,
            allowed_sources,
            model_providers,
            default_provider,
            true,
            search_term,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn list_threads_with_db_fallback(
        config: &Config,
        page_size: usize,
        cursor: Option<&Cursor>,
        sort_key: ThreadSortKey,
        allowed_sources: &[SessionSource],
        model_providers: Option<&[String]>,
        default_provider: &str,
        archived: bool,
        search_term: Option<&str>,
    ) -> std::io::Result<ThreadsPage> {
        let codex_home = config.codex_home.as_path();
        // Filesystem-first listing intentionally overfetches so we can repair stale/missing
        // SQLite rollout paths before the final DB-backed page is returned.
        let fs_page_size = page_size.saturating_mul(2).max(page_size);
        let fs_page = if archived {
            let root = codex_home.join(ARCHIVED_SESSIONS_SUBDIR);
            get_threads_in_root(
                root,
                fs_page_size,
                cursor,
                sort_key,
                ThreadListConfig {
                    allowed_sources,
                    model_providers,
                    default_provider,
                    layout: ThreadListLayout::Flat,
                },
            )
            .await?
        } else {
            get_threads(
                codex_home,
                fs_page_size,
                cursor,
                sort_key,
                allowed_sources,
                model_providers,
                default_provider,
            )
            .await?
        };

        let state_db_ctx = state_db::get_state_db(config, None).await;
        if state_db_ctx.is_none() {
            // Keep legacy behavior when SQLite is unavailable: return filesystem results
            // at the requested page size.
            return Ok(truncate_fs_page(fs_page, page_size, sort_key));
        }

        // Warm the DB by repairing every filesystem hit before querying SQLite.
        for item in &fs_page.items {
            state_db::read_repair_rollout_path(
                state_db_ctx.as_deref(),
                item.thread_id,
                Some(archived),
                item.path.as_path(),
            )
            .await;
        }

        if let Some(db_page) = state_db::list_threads_db(
            state_db_ctx.as_deref(),
            codex_home,
            page_size,
            cursor,
            sort_key,
            allowed_sources,
            model_providers,
            archived,
            search_term,
        )
        .await
        {
            return Ok(db_page.into());
        }
        // If SQLite listing still fails, return the filesystem page rather than failing the list.
        tracing::error!("Falling back on rollout system");
        state_db::record_discrepancy("list_threads_with_db_fallback", "falling_back");
        Ok(truncate_fs_page(fs_page, page_size, sort_key))
    }

    /// Find the newest recorded thread path, optionally filtering to a matching cwd.
    #[allow(clippy::too_many_arguments)]
    pub async fn find_latest_thread_path(
        config: &Config,
        page_size: usize,
        cursor: Option<&Cursor>,
        sort_key: ThreadSortKey,
        allowed_sources: &[SessionSource],
        model_providers: Option<&[String]>,
        default_provider: &str,
        filter_cwd: Option<&Path>,
    ) -> std::io::Result<Option<PathBuf>> {
        let codex_home = config.codex_home.as_path();
        let state_db_ctx = state_db::get_state_db(config, None).await;
        if state_db_ctx.is_some() {
            let mut db_cursor = cursor.cloned();
            loop {
                let Some(db_page) = state_db::list_threads_db(
                    state_db_ctx.as_deref(),
                    codex_home,
                    page_size,
                    db_cursor.as_ref(),
                    sort_key,
                    allowed_sources,
                    model_providers,
                    false,
                    None,
                )
                .await
                else {
                    break;
                };
                if let Some(path) =
                    select_resume_path_from_db_page(&db_page, filter_cwd, default_provider).await
                {
                    return Ok(Some(path));
                }
                db_cursor = db_page.next_anchor.map(Into::into);
                if db_cursor.is_none() {
                    break;
                }
            }
        }

        let mut cursor = cursor.cloned();
        loop {
            let page = get_threads(
                codex_home,
                page_size,
                cursor.as_ref(),
                sort_key,
                allowed_sources,
                model_providers,
                default_provider,
            )
            .await?;
            if let Some(path) = select_resume_path(&page, filter_cwd, default_provider).await {
                return Ok(Some(path));
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                return Ok(None);
            }
        }
    }

    /// Attempt to create a new [`RolloutRecorder`].
    ///
    /// For newly created sessions, this precomputes path/metadata and defers
    /// file creation/open until an explicit `persist()` call.
    ///
    /// For resumed sessions, this immediately opens the existing rollout file.
    pub async fn new(
        config: &Config,
        params: RolloutRecorderParams,
        state_db_ctx: Option<StateDbHandle>,
        state_builder: Option<ThreadMetadataBuilder>,
    ) -> std::io::Result<Self> {
        let (file, deferred_log_file_info, rollout_path, meta, event_persistence_mode) =
            match params {
                RolloutRecorderParams::Create {
                    conversation_id,
                    forked_from_id,
                    source,
                    base_instructions,
                    dynamic_tools,
                    event_persistence_mode,
                } => {
                    let log_file_info = precompute_log_file_info(config, conversation_id)?;
                    let path = log_file_info.path.clone();
                    let session_id = log_file_info.conversation_id;
                    let started_at = log_file_info.timestamp;

                    let timestamp_format: &[FormatItem] = format_description!(
                        "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
                    );
                    let timestamp = started_at
                        .to_offset(time::UtcOffset::UTC)
                        .format(timestamp_format)
                        .map_err(|e| IoError::other(format!("failed to format timestamp: {e}")))?;

                    let session_meta = SessionMeta {
                        id: session_id,
                        forked_from_id,
                        timestamp,
                        cwd: config.cwd.clone(),
                        originator: originator().value,
                        cli_version: env!("CARGO_PKG_VERSION").to_string(),
                        agent_nickname: source.get_nickname(),
                        agent_role: source.get_agent_role(),
                        source,
                        model_provider: Some(config.model_provider_id.clone()),
                        base_instructions: Some(base_instructions),
                        dynamic_tools: if dynamic_tools.is_empty() {
                            None
                        } else {
                            Some(dynamic_tools)
                        },
                    };

                    (
                        None,
                        Some(log_file_info),
                        path,
                        Some(session_meta),
                        event_persistence_mode,
                    )
                }
                RolloutRecorderParams::Resume {
                    path,
                    event_persistence_mode,
                } => (
                    Some(
                        tokio::fs::OpenOptions::new()
                            .append(true)
                            .open(&path)
                            .await?,
                    ),
                    None,
                    path,
                    None,
                    event_persistence_mode,
                ),
            };

        // Clone the cwd for the spawned task to collect git info asynchronously
        let cwd = config.cwd.clone();

        // A reasonably-sized bounded channel. If the buffer fills up the send
        // future will yield, which is fine – we only need to ensure we do not
        // perform *blocking* I/O on the caller's thread.
        let (tx, rx) = mpsc::channel::<RolloutCmd>(256);

        // Spawn a Tokio task that owns the file handle and performs async
        // writes. Using `tokio::fs::File` keeps everything on the async I/O
        // driver instead of blocking the runtime.
        tokio::task::spawn(rollout_writer(
            file,
            deferred_log_file_info,
            rx,
            meta,
            cwd,
            rollout_path.clone(),
            state_db_ctx.clone(),
            state_builder,
            config.model_provider_id.clone(),
            config.memories.generate_memories,
        ));

        Ok(Self {
            tx,
            rollout_path,
            state_db: state_db_ctx,
            event_persistence_mode,
        })
    }

    pub fn rollout_path(&self) -> &Path {
        self.rollout_path.as_path()
    }

    pub fn state_db(&self) -> Option<StateDbHandle> {
        self.state_db.clone()
    }

    pub(crate) async fn record_items(&self, items: &[RolloutItem]) -> std::io::Result<()> {
        let mut filtered = Vec::new();
        for item in items {
            // Note that function calls may look a bit strange if they are
            // "fully qualified MCP tool calls," so we could consider
            // reformatting them in that case.
            if is_persisted_response_item(item, self.event_persistence_mode) {
                filtered.push(sanitize_rollout_item_for_persistence(
                    item.clone(),
                    self.event_persistence_mode,
                ));
            }
        }
        if filtered.is_empty() {
            return Ok(());
        }
        self.tx
            .send(RolloutCmd::AddItems(filtered))
            .await
            .map_err(|e| IoError::other(format!("failed to queue rollout items: {e}")))
    }

    /// Materialize the rollout file and persist all buffered items.
    ///
    /// This is idempotent; after first materialization, repeated calls are no-ops.
    pub async fn persist(&self) -> std::io::Result<()> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(RolloutCmd::Persist { ack: tx })
            .await
            .map_err(|e| IoError::other(format!("failed to queue rollout persist: {e}")))?;
        rx.await
            .map_err(|e| IoError::other(format!("failed waiting for rollout persist: {e}")))
    }

    /// Flush all queued writes and wait until they are committed by the writer task.
    pub async fn flush(&self) -> std::io::Result<()> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(RolloutCmd::Flush { ack: tx })
            .await
            .map_err(|e| IoError::other(format!("failed to queue rollout flush: {e}")))?;
        rx.await
            .map_err(|e| IoError::other(format!("failed waiting for rollout flush: {e}")))
    }

    pub(crate) async fn load_rollout_items(
        path: &Path,
    ) -> std::io::Result<(Vec<RolloutItem>, Option<ThreadId>, usize)> {
        trace!("Resuming rollout from {path:?}");
        let text = tokio::fs::read_to_string(path).await?;
        if text.trim().is_empty() {
            return Err(IoError::other("empty session file"));
        }

        let mut items: Vec<RolloutItem> = Vec::new();
        let mut thread_id: Option<ThreadId> = None;
        let mut parse_errors = 0usize;
        for line in text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let v: Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(e) => {
                    warn!("failed to parse line as JSON: {line:?}, error: {e}");
                    parse_errors = parse_errors.saturating_add(1);
                    continue;
                }
            };

            // Parse the rollout line structure
            match serde_json::from_value::<RolloutLine>(v.clone()) {
                Ok(rollout_line) => match rollout_line.item {
                    RolloutItem::SessionMeta(session_meta_line) => {
                        // Use the FIRST SessionMeta encountered in the file as the canonical
                        // thread id and main session information. Keep all items intact.
                        if thread_id.is_none() {
                            thread_id = Some(session_meta_line.meta.id);
                        }
                        items.push(RolloutItem::SessionMeta(session_meta_line));
                    }
                    RolloutItem::ResponseItem(item) => {
                        items.push(RolloutItem::ResponseItem(item));
                    }
                    RolloutItem::Compacted(item) => {
                        items.push(RolloutItem::Compacted(item));
                    }
                    RolloutItem::TurnContext(item) => {
                        items.push(RolloutItem::TurnContext(item));
                    }
                    RolloutItem::EventMsg(_ev) => {
                        items.push(RolloutItem::EventMsg(_ev));
                    }
                },
                Err(e) => {
                    trace!("failed to parse rollout line: {e}");
                    parse_errors = parse_errors.saturating_add(1);
                }
            }
        }

        tracing::debug!(
            "Resumed rollout with {} items, thread ID: {:?}, parse errors: {}",
            items.len(),
            thread_id,
            parse_errors,
        );
        Ok((items, thread_id, parse_errors))
    }

    pub async fn get_rollout_history(path: &Path) -> std::io::Result<InitialHistory> {
        let (items, thread_id, _parse_errors) = Self::load_rollout_items(path).await?;
        let conversation_id = thread_id
            .ok_or_else(|| IoError::other("failed to parse thread ID from rollout file"))?;

        if items.is_empty() {
            return Ok(InitialHistory::New);
        }

        info!("Resumed rollout successfully from {path:?}");
        Ok(InitialHistory::Resumed(ResumedHistory {
            conversation_id,
            history: items,
            rollout_path: path.to_path_buf(),
        }))
    }

    pub async fn shutdown(&self) -> std::io::Result<()> {
        let (tx_done, rx_done) = oneshot::channel();
        match self.tx.send(RolloutCmd::Shutdown { ack: tx_done }).await {
            Ok(_) => rx_done
                .await
                .map_err(|e| IoError::other(format!("failed waiting for rollout shutdown: {e}"))),
            Err(e) => {
                warn!("failed to send rollout shutdown command: {e}");
                Err(IoError::other(format!(
                    "failed to send rollout shutdown command: {e}"
                )))
            }
        }
    }
}

fn truncate_fs_page(
    mut page: ThreadsPage,
    page_size: usize,
    sort_key: ThreadSortKey,
) -> ThreadsPage {
    if page.items.len() <= page_size {
        return page;
    }
    page.items.truncate(page_size);
    page.next_cursor = page.items.last().and_then(|item| {
        let file_name = item.path.file_name()?.to_str()?;
        let (created_at, id) = parse_timestamp_uuid_from_filename(file_name)?;
        let cursor_token = match sort_key {
            ThreadSortKey::CreatedAt => format!("{}|{id}", created_at.format(&Rfc3339).ok()?),
            ThreadSortKey::UpdatedAt => format!("{}|{id}", item.updated_at.as_deref()?),
        };
        parse_cursor(cursor_token.as_str())
    });
    page
}

struct LogFileInfo {
    /// Full path to the rollout file.
    path: PathBuf,

    /// Session ID (also embedded in filename).
    conversation_id: ThreadId,

    /// Timestamp for the start of the session.
    timestamp: OffsetDateTime,
}

fn precompute_log_file_info(
    config: &Config,
    conversation_id: ThreadId,
) -> std::io::Result<LogFileInfo> {
    // Resolve ~/.codex/sessions/YYYY/MM/DD path.
    let timestamp = OffsetDateTime::now_local()
        .map_err(|e| IoError::other(format!("failed to get local time: {e}")))?;
    let mut dir = config.codex_home.clone();
    dir.push(SESSIONS_SUBDIR);
    dir.push(timestamp.year().to_string());
    dir.push(format!("{:02}", u8::from(timestamp.month())));
    dir.push(format!("{:02}", timestamp.day()));

    // Custom format for YYYY-MM-DDThh-mm-ss. Use `-` instead of `:` for
    // compatibility with filesystems that do not allow colons in filenames.
    let format: &[FormatItem] =
        format_description!("[year]-[month]-[day]T[hour]-[minute]-[second]");
    let date_str = timestamp
        .format(format)
        .map_err(|e| IoError::other(format!("failed to format timestamp: {e}")))?;

    let filename = format!("rollout-{date_str}-{conversation_id}.jsonl");

    let path = dir.join(filename);

    Ok(LogFileInfo {
        path,
        conversation_id,
        timestamp,
    })
}

fn open_log_file(path: &Path) -> std::io::Result<File> {
    let Some(parent) = path.parent() else {
        return Err(IoError::other(format!(
            "rollout path has no parent: {}",
            path.display()
        )));
    };
    fs::create_dir_all(parent)?;
    std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
}

#[allow(clippy::too_many_arguments)]
async fn rollout_writer(
    file: Option<tokio::fs::File>,
    mut deferred_log_file_info: Option<LogFileInfo>,
    mut rx: mpsc::Receiver<RolloutCmd>,
    mut meta: Option<SessionMeta>,
    cwd: std::path::PathBuf,
    rollout_path: PathBuf,
    state_db_ctx: Option<StateDbHandle>,
    mut state_builder: Option<ThreadMetadataBuilder>,
    default_provider: String,
    generate_memories: bool,
) -> std::io::Result<()> {
    let mut writer = file.map(|file| JsonlWriter { file });
    let mut buffered_items = Vec::<RolloutItem>::new();
    let mut agentcanvas_summary_accumulator = AgentCanvasSummaryAccumulator::default();
    if let Some(builder) = state_builder.as_mut() {
        builder.rollout_path = rollout_path.clone();
    }
    agentcanvas_summary_accumulator
        .update_thread_id(state_builder.as_ref(), rollout_path.as_path());

    // Resumed sessions already have a file handle open, so session metadata can
    // be written immediately if present.
    if writer.is_some()
        && let Some(session_meta) = meta.take()
    {
        write_session_meta(
            writer.as_mut(),
            session_meta,
            &cwd,
            &rollout_path,
            state_db_ctx.as_deref(),
            &mut state_builder,
            default_provider.as_str(),
            generate_memories,
        )
        .await?;
        agentcanvas_summary_accumulator
            .update_thread_id(state_builder.as_ref(), rollout_path.as_path());
    }

    // Process rollout commands
    while let Some(cmd) = rx.recv().await {
        match cmd {
            RolloutCmd::AddItems(items) => {
                let mut persisted_items = Vec::new();
                for item in items {
                    persisted_items.push(item);
                }
                if persisted_items.is_empty() {
                    continue;
                }

                if writer.is_none() {
                    buffered_items.extend(persisted_items);
                    continue;
                }

                write_and_reconcile_items(
                    writer.as_mut(),
                    persisted_items.as_slice(),
                    &rollout_path,
                    state_db_ctx.as_deref(),
                    &mut state_builder,
                    default_provider.as_str(),
                    &mut agentcanvas_summary_accumulator,
                )
                .await?;
            }
            RolloutCmd::Persist { ack } => {
                if writer.is_none() {
                    let result = async {
                        let Some(log_file_info) = deferred_log_file_info.take() else {
                            return Err(IoError::other(
                                "deferred rollout recorder missing log file metadata",
                            ));
                        };
                        let file = open_log_file(log_file_info.path.as_path())?;
                        writer = Some(JsonlWriter {
                            file: tokio::fs::File::from_std(file),
                        });

                        if let Some(session_meta) = meta.take() {
                            write_session_meta(
                                writer.as_mut(),
                                session_meta,
                                &cwd,
                                &rollout_path,
                                state_db_ctx.as_deref(),
                                &mut state_builder,
                                default_provider.as_str(),
                                generate_memories,
                            )
                            .await?;
                            agentcanvas_summary_accumulator
                                .update_thread_id(state_builder.as_ref(), rollout_path.as_path());
                        }

                        if !buffered_items.is_empty() {
                            write_and_reconcile_items(
                                writer.as_mut(),
                                buffered_items.as_slice(),
                                &rollout_path,
                                state_db_ctx.as_deref(),
                                &mut state_builder,
                                default_provider.as_str(),
                                &mut agentcanvas_summary_accumulator,
                            )
                            .await?;
                            buffered_items.clear();
                        }

                        Ok(())
                    }
                    .await;

                    if let Err(err) = result {
                        let _ = ack.send(());
                        return Err(err);
                    }
                }
                let _ = ack.send(());
            }
            RolloutCmd::Flush { ack } => {
                // Deferred fresh threads may not have an initialized file yet.
                if let Some(writer) = writer.as_mut()
                    && let Err(e) = writer.file.flush().await
                {
                    let _ = ack.send(());
                    return Err(e);
                }
                let _ = ack.send(());
            }
            RolloutCmd::Shutdown { ack } => {
                let _ = ack.send(());
            }
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn write_session_meta(
    mut writer: Option<&mut JsonlWriter>,
    session_meta: SessionMeta,
    cwd: &Path,
    rollout_path: &Path,
    state_db_ctx: Option<&StateRuntime>,
    state_builder: &mut Option<ThreadMetadataBuilder>,
    default_provider: &str,
    generate_memories: bool,
) -> std::io::Result<()> {
    let git_info = collect_git_info(cwd).await;
    let session_meta_line = SessionMetaLine {
        meta: session_meta,
        git: git_info,
    };
    if state_db_ctx.is_some() {
        *state_builder = metadata::builder_from_session_meta(&session_meta_line, rollout_path);
    }

    let rollout_item = RolloutItem::SessionMeta(session_meta_line);
    if let Some(writer) = writer.as_mut() {
        writer.write_rollout_item(&rollout_item).await?;
    }
    state_db::reconcile_rollout(
        state_db_ctx,
        rollout_path,
        default_provider,
        state_builder.as_ref(),
        std::slice::from_ref(&rollout_item),
        None,
        (!generate_memories).then_some("disabled"),
    )
    .await;
    Ok(())
}

async fn write_and_reconcile_items(
    mut writer: Option<&mut JsonlWriter>,
    items: &[RolloutItem],
    rollout_path: &Path,
    state_db_ctx: Option<&StateRuntime>,
    state_builder: &mut Option<ThreadMetadataBuilder>,
    default_provider: &str,
    agentcanvas_summary_accumulator: &mut AgentCanvasSummaryAccumulator,
) -> std::io::Result<()> {
    if let Some(writer) = writer.as_mut() {
        for item in items {
            writer.write_rollout_item(item).await?;
        }
    }
    if let Some(builder) = state_builder.as_mut() {
        builder.rollout_path = rollout_path.to_path_buf();
    }
    state_db::apply_rollout_items(
        state_db_ctx,
        rollout_path,
        default_provider,
        state_builder.as_ref(),
        items,
        "rollout_writer",
        None,
    )
    .await;
    agentcanvas_summary_accumulator.update_thread_id(state_builder.as_ref(), rollout_path);
    let summary_updates = agentcanvas_summary_accumulator.handle_rollout_items(items);
    persist_agentcanvas_turn_summaries(
        state_db_ctx,
        agentcanvas_summary_accumulator,
        summary_updates,
    )
    .await;
    Ok(())
}

struct JsonlWriter {
    file: tokio::fs::File,
}

#[derive(serde::Serialize)]
struct RolloutLineRef<'a> {
    timestamp: String,
    #[serde(flatten)]
    item: &'a RolloutItem,
}

impl JsonlWriter {
    async fn write_rollout_item(&mut self, rollout_item: &RolloutItem) -> std::io::Result<()> {
        let timestamp_format: &[FormatItem] = format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
        );
        let timestamp = OffsetDateTime::now_utc()
            .format(timestamp_format)
            .map_err(|e| IoError::other(format!("failed to format timestamp: {e}")))?;

        let line = RolloutLineRef {
            timestamp,
            item: rollout_item,
        };
        self.write_line(&line).await
    }
    async fn write_line(&mut self, item: &impl serde::Serialize) -> std::io::Result<()> {
        let mut json = serde_json::to_string(item)?;
        json.push('\n');
        self.file.write_all(json.as_bytes()).await?;
        self.file.flush().await?;
        Ok(())
    }
}

impl From<codex_state::ThreadsPage> for ThreadsPage {
    fn from(db_page: codex_state::ThreadsPage) -> Self {
        let items = db_page
            .items
            .into_iter()
            .map(|item| ThreadItem {
                path: item.rollout_path,
                thread_id: Some(item.id),
                first_user_message: item.first_user_message,
                cwd: Some(item.cwd),
                git_branch: item.git_branch,
                git_sha: item.git_sha,
                git_origin_url: item.git_origin_url,
                source: Some(
                    serde_json::from_str(item.source.as_str())
                        .or_else(|_| serde_json::from_value(Value::String(item.source)))
                        .unwrap_or(SessionSource::Unknown),
                ),
                agent_nickname: item.agent_nickname,
                agent_role: item.agent_role,
                model_provider: Some(item.model_provider),
                cli_version: Some(item.cli_version),
                created_at: Some(item.created_at.to_rfc3339_opts(SecondsFormat::Secs, true)),
                updated_at: Some(item.updated_at.to_rfc3339_opts(SecondsFormat::Secs, true)),
            })
            .collect();
        Self {
            items,
            next_cursor: db_page.next_anchor.map(Into::into),
            num_scanned_files: db_page.num_scanned_rows,
            reached_scan_cap: false,
        }
    }
}

async fn select_resume_path(
    page: &ThreadsPage,
    filter_cwd: Option<&Path>,
    default_provider: &str,
) -> Option<PathBuf> {
    match filter_cwd {
        Some(cwd) => {
            for item in &page.items {
                if resume_candidate_matches_cwd(
                    item.path.as_path(),
                    item.cwd.as_deref(),
                    cwd,
                    default_provider,
                )
                .await
                {
                    return Some(item.path.clone());
                }
            }
            None
        }
        None => page.items.first().map(|item| item.path.clone()),
    }
}

async fn resume_candidate_matches_cwd(
    rollout_path: &Path,
    cached_cwd: Option<&Path>,
    cwd: &Path,
    default_provider: &str,
) -> bool {
    if cached_cwd.is_some_and(|session_cwd| cwd_matches(session_cwd, cwd)) {
        return true;
    }

    if let Ok((items, _, _)) = RolloutRecorder::load_rollout_items(rollout_path).await
        && let Some(latest_turn_context_cwd) = items.iter().rev().find_map(|item| match item {
            RolloutItem::TurnContext(turn_context) => Some(turn_context.cwd.as_path()),
            RolloutItem::SessionMeta(_)
            | RolloutItem::ResponseItem(_)
            | RolloutItem::Compacted(_)
            | RolloutItem::EventMsg(_) => None,
        })
    {
        return cwd_matches(latest_turn_context_cwd, cwd);
    }

    metadata::extract_metadata_from_rollout(rollout_path, default_provider, None)
        .await
        .is_ok_and(|outcome| cwd_matches(outcome.metadata.cwd.as_path(), cwd))
}

async fn select_resume_path_from_db_page(
    page: &codex_state::ThreadsPage,
    filter_cwd: Option<&Path>,
    default_provider: &str,
) -> Option<PathBuf> {
    match filter_cwd {
        Some(cwd) => {
            for item in &page.items {
                if resume_candidate_matches_cwd(
                    item.rollout_path.as_path(),
                    Some(item.cwd.as_path()),
                    cwd,
                    default_provider,
                )
                .await
                {
                    return Some(item.rollout_path.clone());
                }
            }
            None
        }
        None => page.items.first().map(|item| item.rollout_path.clone()),
    }
}

fn cwd_matches(session_cwd: &Path, cwd: &Path) -> bool {
    if let (Ok(ca), Ok(cb)) = (
        path_utils::normalize_for_path_comparison(session_cwd),
        path_utils::normalize_for_path_comparison(cwd),
    ) {
        return ca == cb;
    }
    session_cwd == cwd
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigBuilder;
    use crate::features::Feature;
    use chrono::TimeZone;
    use codex_protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
    use codex_protocol::models::LocalShellAction;
    use codex_protocol::models::LocalShellExecAction;
    use codex_protocol::models::LocalShellStatus;
    use codex_protocol::protocol::AgentMessageEvent;
    use codex_protocol::protocol::AskForApproval;
    use codex_protocol::protocol::ErrorEvent;
    use codex_protocol::protocol::EventMsg;
    use codex_protocol::protocol::SandboxPolicy;
    use codex_protocol::protocol::SessionMeta;
    use codex_protocol::protocol::SessionMetaLine;
    use codex_protocol::protocol::ThreadRolledBackEvent;
    use codex_protocol::protocol::TurnCompleteEvent;
    use codex_protocol::protocol::TurnContextItem;
    use codex_protocol::protocol::TurnStartedEvent;
    use codex_protocol::protocol::UserMessageEvent;
    use codex_protocol::protocol::ViewImageToolCallEvent;
    use pretty_assertions::assert_eq;
    use std::fs::File;
    use std::fs::{self};
    use std::io::Write;
    use std::path::Path;
    use std::path::PathBuf;
    use tempfile::TempDir;
    use uuid::Uuid;

    fn write_session_file(root: &Path, ts: &str, uuid: Uuid) -> std::io::Result<PathBuf> {
        let day_dir = root.join("sessions/2025/01/03");
        fs::create_dir_all(&day_dir)?;
        let path = day_dir.join(format!("rollout-{ts}-{uuid}.jsonl"));
        let mut file = File::create(&path)?;
        let meta = serde_json::json!({
            "timestamp": ts,
            "type": "session_meta",
            "payload": {
                "id": uuid,
                "timestamp": ts,
                "cwd": ".",
                "originator": "test_originator",
                "cli_version": "test_version",
                "source": "cli",
                "model_provider": "test-provider",
            },
        });
        writeln!(file, "{meta}")?;
        let user_event = serde_json::json!({
            "timestamp": ts,
            "type": "event_msg",
            "payload": {
                "type": "user_message",
                "message": "Hello from user",
                "kind": "plain",
            },
        });
        writeln!(file, "{user_event}")?;
        Ok(path)
    }

    fn turn_started_item(turn_id: &str) -> RolloutItem {
        RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
            turn_id: turn_id.to_string(),
            model_context_window: None,
            collaboration_mode_kind: Default::default(),
        }))
    }

    fn turn_complete_item(turn_id: &str, last_agent_message: Option<&str>) -> RolloutItem {
        RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: turn_id.to_string(),
            last_agent_message: last_agent_message.map(ToOwned::to_owned),
        }))
    }

    #[tokio::test]
    async fn recorder_materializes_only_after_explicit_persist() -> std::io::Result<()> {
        let home = TempDir::new().expect("temp dir");
        let config = ConfigBuilder::default()
            .codex_home(home.path().to_path_buf())
            .build()
            .await?;
        let thread_id = ThreadId::new();
        let recorder = RolloutRecorder::new(
            &config,
            RolloutRecorderParams::new(
                thread_id,
                None,
                SessionSource::Exec,
                BaseInstructions::default(),
                Vec::new(),
                EventPersistenceMode::Limited,
            ),
            None,
            None,
        )
        .await?;

        let rollout_path = recorder.rollout_path().to_path_buf();
        assert!(
            !rollout_path.exists(),
            "rollout file should not exist before first user message"
        );

        recorder
            .record_items(&[RolloutItem::EventMsg(EventMsg::AgentMessage(
                AgentMessageEvent {
                    message: "buffered-event".to_string(),
                    phase: None,
                },
            ))])
            .await?;
        recorder.flush().await?;
        assert!(
            !rollout_path.exists(),
            "rollout file should remain deferred before first user message"
        );

        recorder
            .record_items(&[RolloutItem::EventMsg(EventMsg::UserMessage(
                UserMessageEvent {
                    message: "first-user-message".to_string(),
                    images: None,
                    local_images: Vec::new(),
                    text_elements: Vec::new(),
                },
            ))])
            .await?;
        recorder.flush().await?;
        assert!(
            !rollout_path.exists(),
            "user-message-like items should not materialize without explicit persist"
        );

        recorder.persist().await?;
        // Second call verifies `persist()` is idempotent after materialization.
        recorder.persist().await?;
        assert!(rollout_path.exists(), "rollout file should be materialized");

        let text = std::fs::read_to_string(&rollout_path)?;
        assert!(
            text.contains("\"type\":\"session_meta\""),
            "expected session metadata in rollout"
        );
        let buffered_idx = text
            .find("buffered-event")
            .expect("buffered event in rollout");
        let user_idx = text
            .find("first-user-message")
            .expect("first user message in rollout");
        assert!(
            buffered_idx < user_idx,
            "buffered items should preserve ordering"
        );
        let text_after_second_persist = std::fs::read_to_string(&rollout_path)?;
        assert_eq!(text_after_second_persist, text);

        recorder.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn list_threads_db_disabled_does_not_skip_paginated_items() -> std::io::Result<()> {
        let home = TempDir::new().expect("temp dir");
        let mut config = ConfigBuilder::default()
            .codex_home(home.path().to_path_buf())
            .build()
            .await?;
        config.features.disable(Feature::Sqlite);

        let newest = write_session_file(home.path(), "2025-01-03T12-00-00", Uuid::from_u128(9001))?;
        let middle = write_session_file(home.path(), "2025-01-02T12-00-00", Uuid::from_u128(9002))?;
        let _oldest =
            write_session_file(home.path(), "2025-01-01T12-00-00", Uuid::from_u128(9003))?;

        let default_provider = config.model_provider_id.clone();
        let page1 = RolloutRecorder::list_threads(
            &config,
            1,
            None,
            ThreadSortKey::CreatedAt,
            &[],
            None,
            default_provider.as_str(),
            None,
        )
        .await?;
        assert_eq!(page1.items.len(), 1);
        assert_eq!(page1.items[0].path, newest);
        let cursor = page1.next_cursor.clone().expect("cursor should be present");

        let page2 = RolloutRecorder::list_threads(
            &config,
            1,
            Some(&cursor),
            ThreadSortKey::CreatedAt,
            &[],
            None,
            default_provider.as_str(),
            None,
        )
        .await?;
        assert_eq!(page2.items.len(), 1);
        assert_eq!(page2.items[0].path, middle);
        Ok(())
    }

    #[tokio::test]
    async fn list_threads_db_enabled_drops_missing_rollout_paths() -> std::io::Result<()> {
        let home = TempDir::new().expect("temp dir");
        let mut config = ConfigBuilder::default()
            .codex_home(home.path().to_path_buf())
            .build()
            .await?;
        config.features.enable(Feature::Sqlite);

        let uuid = Uuid::from_u128(9010);
        let thread_id = ThreadId::from_string(&uuid.to_string()).expect("valid thread id");
        let stale_path = home.path().join(format!(
            "sessions/2099/01/01/rollout-2099-01-01T00-00-00-{uuid}.jsonl"
        ));

        let runtime = codex_state::StateRuntime::init(
            home.path().to_path_buf(),
            config.model_provider_id.clone(),
            None,
        )
        .await
        .expect("state db should initialize");
        runtime
            .mark_backfill_complete(None)
            .await
            .expect("backfill should be complete");
        let created_at = chrono::Utc
            .with_ymd_and_hms(2025, 1, 3, 13, 0, 0)
            .single()
            .expect("valid datetime");
        let mut builder = codex_state::ThreadMetadataBuilder::new(
            thread_id,
            stale_path,
            created_at,
            SessionSource::Cli,
        );
        builder.model_provider = Some(config.model_provider_id.clone());
        builder.cwd = home.path().to_path_buf();
        let mut metadata = builder.build(config.model_provider_id.as_str());
        metadata.first_user_message = Some("Hello from user".to_string());
        runtime
            .upsert_thread(&metadata)
            .await
            .expect("state db upsert should succeed");

        let default_provider = config.model_provider_id.clone();
        let page = RolloutRecorder::list_threads(
            &config,
            10,
            None,
            ThreadSortKey::CreatedAt,
            &[],
            None,
            default_provider.as_str(),
            None,
        )
        .await?;
        assert_eq!(page.items.len(), 0);
        let stored_path = runtime
            .find_rollout_path_by_id(thread_id, Some(false))
            .await
            .expect("state db lookup should succeed");
        assert_eq!(stored_path, None);
        Ok(())
    }

    #[tokio::test]
    async fn list_threads_db_enabled_repairs_stale_rollout_paths() -> std::io::Result<()> {
        let home = TempDir::new().expect("temp dir");
        let mut config = ConfigBuilder::default()
            .codex_home(home.path().to_path_buf())
            .build()
            .await?;
        config.features.enable(Feature::Sqlite);

        let uuid = Uuid::from_u128(9011);
        let thread_id = ThreadId::from_string(&uuid.to_string()).expect("valid thread id");
        let real_path = write_session_file(home.path(), "2025-01-03T13-00-00", uuid)?;
        let stale_path = home.path().join(format!(
            "sessions/2099/01/01/rollout-2099-01-01T00-00-00-{uuid}.jsonl"
        ));

        let runtime = codex_state::StateRuntime::init(
            home.path().to_path_buf(),
            config.model_provider_id.clone(),
            None,
        )
        .await
        .expect("state db should initialize");
        runtime
            .mark_backfill_complete(None)
            .await
            .expect("backfill should be complete");
        let created_at = chrono::Utc
            .with_ymd_and_hms(2025, 1, 3, 13, 0, 0)
            .single()
            .expect("valid datetime");
        let mut builder = codex_state::ThreadMetadataBuilder::new(
            thread_id,
            stale_path,
            created_at,
            SessionSource::Cli,
        );
        builder.model_provider = Some(config.model_provider_id.clone());
        builder.cwd = home.path().to_path_buf();
        let mut metadata = builder.build(config.model_provider_id.as_str());
        metadata.first_user_message = Some("Hello from user".to_string());
        runtime
            .upsert_thread(&metadata)
            .await
            .expect("state db upsert should succeed");

        let default_provider = config.model_provider_id.clone();
        let page = RolloutRecorder::list_threads(
            &config,
            1,
            None,
            ThreadSortKey::CreatedAt,
            &[],
            None,
            default_provider.as_str(),
            None,
        )
        .await?;
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].path, real_path);

        let repaired_path = runtime
            .find_rollout_path_by_id(thread_id, Some(false))
            .await
            .expect("state db lookup should succeed");
        assert_eq!(repaired_path, Some(real_path));
        Ok(())
    }

    #[tokio::test]
    async fn resume_candidate_matches_cwd_reads_latest_turn_context() -> std::io::Result<()> {
        let home = TempDir::new().expect("temp dir");
        let stale_cwd = home.path().join("stale");
        let latest_cwd = home.path().join("latest");
        fs::create_dir_all(&stale_cwd)?;
        fs::create_dir_all(&latest_cwd)?;

        let path = write_session_file(home.path(), "2025-01-03T13-00-00", Uuid::from_u128(9012))?;
        let mut file = std::fs::OpenOptions::new().append(true).open(&path)?;
        let turn_context = RolloutLine {
            timestamp: "2025-01-03T13:00:01Z".to_string(),
            item: RolloutItem::TurnContext(TurnContextItem {
                turn_id: Some("turn-1".to_string()),
                cwd: latest_cwd.clone(),
                current_date: None,
                timezone: None,
                approval_policy: AskForApproval::Never,
                sandbox_policy: SandboxPolicy::new_read_only_policy(),
                network: None,
                model: "test-model".to_string(),
                personality: None,
                collaboration_mode: None,
                effort: None,
                summary: ReasoningSummaryConfig::Auto,
                user_instructions: None,
                developer_instructions: None,
                final_output_json_schema: None,
                truncation_policy: None,
            }),
        };
        writeln!(file, "{}", serde_json::to_string(&turn_context)?)?;

        assert!(
            resume_candidate_matches_cwd(
                path.as_path(),
                Some(stale_cwd.as_path()),
                latest_cwd.as_path(),
                "test-provider",
            )
            .await
        );
        Ok(())
    }

    #[tokio::test]
    async fn recorder_persists_agentcanvas_turn_summary_on_turn_complete() -> std::io::Result<()> {
        let home = TempDir::new().expect("temp dir");
        let mut config = ConfigBuilder::default()
            .codex_home(home.path().to_path_buf())
            .build()
            .await?;
        config.features.enable(Feature::Sqlite);

        let state_db = codex_state::StateRuntime::init(
            home.path().to_path_buf(),
            config.model_provider_id.clone(),
            None,
        )
        .await
        .expect("state db should initialize");

        let thread_id = ThreadId::new();
        let turn_id = "turn-1".to_string();
        let recorder = RolloutRecorder::new(
            &config,
            RolloutRecorderParams::new(
                thread_id,
                None,
                SessionSource::Cli,
                BaseInstructions::default(),
                Vec::new(),
                EventPersistenceMode::Limited,
            ),
            Some(state_db.clone()),
            None,
        )
        .await?;

        recorder
            .record_items(&[
                turn_started_item(turn_id.as_str()),
                RolloutItem::ResponseItem(ResponseItem::LocalShellCall {
                    id: None,
                    call_id: Some("call-1".to_string()),
                    status: LocalShellStatus::Completed,
                    action: LocalShellAction::Exec(LocalShellExecAction {
                        command: vec!["rg".to_string(), "summary".to_string()],
                        timeout_ms: None,
                        working_directory: None,
                        env: None,
                        user: None,
                    }),
                }),
                RolloutItem::EventMsg(EventMsg::Error(ErrorEvent {
                    message: "boom".to_string(),
                    codex_error_info: None,
                })),
                turn_complete_item(turn_id.as_str(), Some("completed")),
            ])
            .await?;

        recorder.persist().await?;
        recorder.flush().await?;

        let thread_id_str = thread_id.to_string();
        let artifact = state_db
            .get_session_summary_by_thread_and_session(thread_id_str.as_str(), turn_id.as_str())
            .await
            .expect("summary lookup should succeed")
            .expect("summary should be persisted");
        assert_eq!(artifact.root_node_id, Some(format!("turn:{turn_id}")));

        let summary = state_db
            .read_session_summary_by_thread_and_session(thread_id_str.as_str(), turn_id.as_str())
            .await
            .expect("summary read should succeed")
            .expect("summary payload should exist");
        assert_eq!(
            summary["schema_version"],
            serde_json::json!("agentcanvas.turn.v2")
        );
        assert_eq!(
            summary["summary_kind"],
            serde_json::json!("agentcanvas_turn_summary")
        );
        assert!(summary.get("node_type").is_none());
        assert_eq!(
            summary["thread_id"],
            serde_json::json!(thread_id_str.clone())
        );
        assert_eq!(summary["session_id"], serde_json::json!(turn_id.clone()));
        assert_eq!(
            summary["nodes"][0]["brief"]["signal"],
            serde_json::json!("error")
        );
        assert_eq!(
            summary["nodes"][0]["evidence"]["commands"][0]["command"],
            serde_json::json!("rg summary")
        );
        assert_eq!(
            summary["nodes"][0]["evidence"]["errors"][0],
            serde_json::json!("boom")
        );
        assert_eq!(
            summary["nodes"][0]["counts"]["commands_total"],
            serde_json::json!(1)
        );
        assert_eq!(
            summary["nodes"][0]["counts"]["commands_omitted"],
            serde_json::json!(0)
        );
        assert_eq!(
            summary["nodes"][0]["digest"]["command_examples"][0],
            serde_json::json!("rg summary")
        );
        let summary_text = summary["nodes"][0]["summary"]
            .as_str()
            .expect("summary text should exist");
        assert!(!summary_text.trim().is_empty());

        let command_matches = state_db
            .search_summary_nodes_by_command_substring("rg summary", 10)
            .await
            .expect("command search should succeed");
        assert_eq!(command_matches.len(), 1);
        assert_eq!(command_matches[0].thread_id, thread_id_str);
        assert_eq!(command_matches[0].session_id, turn_id);
        let nodes = state_db
            .list_summary_nodes_by_thread_and_session(thread_id_str.as_str(), turn_id.as_str(), 10)
            .await
            .expect("node list should succeed");
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].node_id, format!("turn:{turn_id}"));

        recorder.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn recorder_compacts_agentcanvas_summary_evidence_for_parseability() -> std::io::Result<()>
    {
        let home = TempDir::new().expect("temp dir");
        let mut config = ConfigBuilder::default()
            .codex_home(home.path().to_path_buf())
            .build()
            .await?;
        config.features.enable(Feature::Sqlite);

        let state_db = codex_state::StateRuntime::init(
            home.path().to_path_buf(),
            config.model_provider_id.clone(),
            None,
        )
        .await
        .expect("state db should initialize");

        let thread_id = ThreadId::new();
        let turn_id = "turn-compact";
        let recorder = RolloutRecorder::new(
            &config,
            RolloutRecorderParams::new(
                thread_id,
                None,
                SessionSource::Cli,
                BaseInstructions::default(),
                Vec::new(),
                EventPersistenceMode::Limited,
            ),
            Some(state_db.clone()),
            None,
        )
        .await?;

        let mut items = vec![turn_started_item(turn_id)];
        for idx in 0..(SUMMARY_MAX_COMMAND_ITEMS + 7) {
            items.push(RolloutItem::ResponseItem(ResponseItem::LocalShellCall {
                id: None,
                call_id: Some(format!("call-{idx}")),
                status: LocalShellStatus::Completed,
                action: LocalShellAction::Exec(LocalShellExecAction {
                    command: vec!["echo".to_string(), format!("cmd-{idx}")],
                    timeout_ms: None,
                    working_directory: None,
                    env: None,
                    user: None,
                }),
            }));
        }
        for idx in 0..(SUMMARY_MAX_FILE_PATH_ITEMS + 3) {
            items.push(RolloutItem::EventMsg(EventMsg::ViewImageToolCall(
                ViewImageToolCallEvent {
                    call_id: format!("view-image-{idx}"),
                    path: PathBuf::from(format!("/repo/src/file-{idx}.rs")),
                },
            )));
        }
        for idx in 0..(SUMMARY_MAX_ERROR_ITEMS + 5) {
            items.push(RolloutItem::EventMsg(EventMsg::Error(ErrorEvent {
                message: format!(
                    "failure-{idx} {}",
                    "x".repeat(SUMMARY_MAX_ERROR_TEXT_BYTES + 120)
                ),
                codex_error_info: None,
            })));
        }
        items.push(turn_complete_item(
            turn_id,
            Some("done ".repeat(SUMMARY_MAX_AGENT_MESSAGE_BYTES).as_str()),
        ));
        recorder.record_items(items.as_slice()).await?;

        recorder.persist().await?;
        recorder.flush().await?;

        let thread_id_str = thread_id.to_string();
        let summary = state_db
            .read_session_summary_by_thread_and_session(thread_id_str.as_str(), turn_id)
            .await
            .expect("summary read should succeed")
            .expect("summary payload should exist");
        let node = &summary["nodes"][0];
        assert_eq!(
            node["counts"]["commands_total"],
            serde_json::json!(SUMMARY_MAX_COMMAND_ITEMS + 7)
        );
        assert_eq!(node["counts"]["commands_omitted"], serde_json::json!(7));
        assert_eq!(
            node["counts"]["file_paths_total"],
            serde_json::json!(SUMMARY_MAX_FILE_PATH_ITEMS + 3)
        );
        assert_eq!(node["counts"]["file_paths_omitted"], serde_json::json!(3));
        assert_eq!(
            node["counts"]["errors_total"],
            serde_json::json!(SUMMARY_MAX_ERROR_ITEMS + 5)
        );
        assert_eq!(node["counts"]["errors_omitted"], serde_json::json!(5));
        assert_eq!(
            node["digest"]["command_examples"]
                .as_array()
                .expect("command digest should be array")
                .len(),
            SUMMARY_DIGEST_ITEMS
        );
        assert_eq!(
            node["digest"]["file_path_examples"]
                .as_array()
                .expect("file path digest should be array")
                .len(),
            SUMMARY_DIGEST_ITEMS
        );
        assert_eq!(
            node["digest"]["error_examples"]
                .as_array()
                .expect("error digest should be array")
                .len(),
            SUMMARY_DIGEST_ITEMS
        );
        assert_eq!(
            node["evidence"]["commands"]
                .as_array()
                .expect("command evidence should be array")
                .len(),
            SUMMARY_MAX_COMMAND_ITEMS
        );
        assert_eq!(
            node["evidence"]["file_paths"]
                .as_array()
                .expect("file path evidence should be array")
                .len(),
            SUMMARY_MAX_FILE_PATH_ITEMS
        );
        assert_eq!(
            node["evidence"]["errors"]
                .as_array()
                .expect("error evidence should be array")
                .len(),
            SUMMARY_MAX_ERROR_ITEMS
        );
        let summary_text = node["summary"].as_str().expect("summary text should exist");
        assert!(!summary_text.trim().is_empty());
        if summary_text.contains("status=") {
            assert!(summary_text.len() <= SUMMARY_MAX_AGENT_MESSAGE_BYTES);
        } else {
            assert!(!summary_text.trim().is_empty());
        }
        assert_eq!(node["brief"]["signal"], serde_json::json!("error"));
        assert!(
            node["brief"]["agent_message"]
                .as_str()
                .expect("brief agent message should exist")
                .len()
                <= SUMMARY_MAX_AGENT_MESSAGE_BYTES
        );

        recorder.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn recorder_persists_turn_lineage_for_forked_threads() -> std::io::Result<()> {
        let home = TempDir::new().expect("temp dir");
        let mut config = ConfigBuilder::default()
            .codex_home(home.path().to_path_buf())
            .build()
            .await?;
        config.features.enable(Feature::Sqlite);

        let state_db = codex_state::StateRuntime::init(
            home.path().to_path_buf(),
            config.model_provider_id.clone(),
            None,
        )
        .await
        .expect("state db should initialize");

        let thread_id = ThreadId::new();
        let forked_from_thread_id = ThreadId::new();
        let recorder = RolloutRecorder::new(
            &config,
            RolloutRecorderParams::new(
                thread_id,
                None,
                SessionSource::Cli,
                BaseInstructions::default(),
                Vec::new(),
                EventPersistenceMode::Limited,
            ),
            Some(state_db.clone()),
            None,
        )
        .await?;

        recorder
            .record_items(&[
                RolloutItem::SessionMeta(SessionMetaLine {
                    meta: SessionMeta {
                        id: thread_id,
                        forked_from_id: Some(forked_from_thread_id),
                        timestamp: "2026-02-28T00:00:00Z".to_string(),
                        cwd: home.path().to_path_buf(),
                        originator: "test_originator".to_string(),
                        cli_version: "test_version".to_string(),
                        source: SessionSource::Cli,
                        agent_nickname: None,
                        agent_role: None,
                        model_provider: Some(config.model_provider_id.clone()),
                        base_instructions: None,
                        dynamic_tools: None,
                    },
                    git: None,
                }),
                turn_started_item("turn-1"),
                turn_complete_item("turn-1", Some("completed-1")),
                turn_started_item("turn-2"),
                turn_complete_item("turn-2", Some("completed-2")),
            ])
            .await?;

        recorder.persist().await?;
        recorder.flush().await?;

        let thread_id_str = thread_id.to_string();
        let forked_from_thread_id_str = forked_from_thread_id.to_string();
        let turn_1_summary = state_db
            .read_session_summary_by_thread_and_session(thread_id_str.as_str(), "turn-1")
            .await
            .expect("turn-1 summary read should succeed")
            .expect("turn-1 summary should be present");
        assert_eq!(
            turn_1_summary["nodes"][0]["parent_id"],
            serde_json::Value::Null
        );
        assert_eq!(
            turn_1_summary["nodes"][0]["lineage"]["forked_from_thread_id"],
            serde_json::json!(forked_from_thread_id_str.clone())
        );
        assert_eq!(
            turn_1_summary["nodes"][0]["lineage"]["child_turn_id"],
            serde_json::json!("turn-2")
        );

        let turn_2_summary = state_db
            .read_session_summary_by_thread_and_session(thread_id_str.as_str(), "turn-2")
            .await
            .expect("turn-2 summary read should succeed")
            .expect("turn-2 summary should be present");
        assert_eq!(
            turn_2_summary["nodes"][0]["parent_id"],
            serde_json::json!("turn:turn-1")
        );
        assert_eq!(
            turn_2_summary["nodes"][0]["lineage"]["parent_turn_id"],
            serde_json::json!("turn-1")
        );
        assert_eq!(
            turn_2_summary["nodes"][0]["lineage"]["forked_from_thread_id"],
            serde_json::json!(forked_from_thread_id_str)
        );
        assert!(turn_2_summary["nodes"][0]["lineage"]["child_turn_id"].is_null());

        recorder.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn recorder_tracks_backtracking_after_thread_rollback() -> std::io::Result<()> {
        let home = TempDir::new().expect("temp dir");
        let mut config = ConfigBuilder::default()
            .codex_home(home.path().to_path_buf())
            .build()
            .await?;
        config.features.enable(Feature::Sqlite);

        let state_db = codex_state::StateRuntime::init(
            home.path().to_path_buf(),
            config.model_provider_id.clone(),
            None,
        )
        .await
        .expect("state db should initialize");

        let thread_id = ThreadId::new();
        let recorder = RolloutRecorder::new(
            &config,
            RolloutRecorderParams::new(
                thread_id,
                None,
                SessionSource::Cli,
                BaseInstructions::default(),
                Vec::new(),
                EventPersistenceMode::Limited,
            ),
            Some(state_db.clone()),
            None,
        )
        .await?;

        recorder
            .record_items(&[
                turn_started_item("turn-a"),
                turn_complete_item("turn-a", Some("completed-a")),
                turn_started_item("turn-b"),
                turn_complete_item("turn-b", Some("completed-b")),
                RolloutItem::EventMsg(EventMsg::ThreadRolledBack(ThreadRolledBackEvent {
                    num_turns: 1,
                })),
                turn_started_item("turn-c"),
                turn_complete_item("turn-c", Some("completed-c")),
            ])
            .await?;

        recorder.persist().await?;
        recorder.flush().await?;

        let thread_id_str = thread_id.to_string();
        let turn_b_summary = state_db
            .read_session_summary_by_thread_and_session(thread_id_str.as_str(), "turn-b")
            .await
            .expect("turn-b summary read should succeed")
            .expect("turn-b summary should be present");
        assert_eq!(
            turn_b_summary["nodes"][0]["status"],
            serde_json::json!("rolled_back")
        );
        assert_eq!(
            turn_b_summary["nodes"][0]["lineage"]["was_rolled_back"],
            serde_json::json!(true)
        );

        let turn_c_summary = state_db
            .read_session_summary_by_thread_and_session(thread_id_str.as_str(), "turn-c")
            .await
            .expect("turn-c summary read should succeed")
            .expect("turn-c summary should be present");
        assert_eq!(
            turn_c_summary["nodes"][0]["parent_id"],
            serde_json::json!("turn:turn-a")
        );
        assert_eq!(
            turn_c_summary["nodes"][0]["lineage"]["parent_turn_id"],
            serde_json::json!("turn-a")
        );
        assert_eq!(
            turn_c_summary["nodes"][0]["lineage"]["started_after_rollback"],
            serde_json::json!(true)
        );

        recorder.shutdown().await?;
        Ok(())
    }
}
