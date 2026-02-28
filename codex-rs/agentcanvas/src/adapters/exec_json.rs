//! Exec JSON adapter for --experimental-json stream

use std::collections::HashMap;
use std::pin::Pin;
use std::task::{Context, Poll};

use codex_exec::exec_events::*;
use futures::stream::Stream;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, BufReader};
use tracing::warn;

use crate::schema::*;
use crate::utils::{current_timestamp, generate_item_id, generate_turn_id};

use super::{AdapterError, EventAdapter};

/// State for an in-progress item
#[derive(Debug, Clone)]
struct ItemState {
    item_type: ItemType,
    details: ItemDetails,
}

/// Adapter for exec --experimental-json JSONL stream
pub struct ExecJsonAdapter<R: AsyncBufRead + Unpin> {
    reader: BufReader<R>,
    buffer: String,
    thread_id: Option<ThreadId>,
    turn_id_counter: usize,
    current_turn_id: Option<TurnId>,
    item_states: HashMap<ItemId, ItemState>,
    pending_events: Vec<NormalizedEvent>,
}

impl<R: AsyncBufRead + Unpin> ExecJsonAdapter<R> {
    /// Create a new exec JSON adapter
    pub fn new(reader: R) -> Self {
        Self {
            reader: BufReader::new(reader),
            buffer: String::new(),
            thread_id: None,
            turn_id_counter: 0,
            current_turn_id: None,
            item_states: HashMap::new(),
            pending_events: Vec::new(),
        }
    }

    fn process_event(&mut self, event: ThreadEvent) -> Result<Option<NormalizedEvent>, AdapterError> {
        let timestamp = current_timestamp();
        let thread_id = self.thread_id.clone().unwrap_or_default();

        match event {
            ThreadEvent::ThreadStarted(ThreadStartedEvent { thread_id: tid }) => {
                self.thread_id = Some(tid.to_string());
                Ok(Some(NormalizedEvent::new(
                    tid.to_string(),
                    None,
                    None,
                    timestamp,
                    EventSource::ExecJson,
                    "thread.started".to_string(),
                    EventPayload::ThreadStarted(ThreadStartedPayload {}),
                )))
            }

            ThreadEvent::TurnStarted(_) => {
                let turn_id = generate_turn_id(self.turn_id_counter);
                self.turn_id_counter += 1;
                self.current_turn_id = Some(turn_id.clone());

                Ok(Some(NormalizedEvent::new(
                    thread_id,
                    Some(turn_id),
                    None,
                    timestamp,
                    EventSource::ExecJson,
                    "turn.started".to_string(),
                    EventPayload::TurnStarted(TurnStartedPayload {}),
                )))
            }

            ThreadEvent::ItemStarted(ItemStartedEvent { item }) => {
                let (item_type, details) = self.map_thread_item(&item)?;
                let item_id = item.id.clone();

                self.item_states.insert(
                    item_id.clone(),
                    ItemState {
                        item_type: item_type.clone(),
                        details: details.clone(),
                    },
                );

                Ok(Some(NormalizedEvent::new(
                    thread_id,
                    self.current_turn_id.clone(),
                    Some(item_id),
                    timestamp,
                    EventSource::ExecJson,
                    "item.started".to_string(),
                    EventPayload::ItemStarted(ItemStartedPayload {
                        item_type,
                        details,
                    }),
                )))
            }

            ThreadEvent::ItemUpdated(ItemUpdatedEvent { item }) => {
                let (item_type, details) = self.map_thread_item(&item)?;
                let item_id = item.id.clone();

                // Update cached state
                self.item_states.insert(
                    item_id.clone(),
                    ItemState {
                        item_type: item_type.clone(),
                        details: details.clone(),
                    },
                );

                Ok(Some(NormalizedEvent::new(
                    thread_id,
                    self.current_turn_id.clone(),
                    Some(item_id),
                    timestamp,
                    EventSource::ExecJson,
                    "item.updated".to_string(),
                    EventPayload::ItemUpdated(ItemUpdatedPayload {
                        item_type,
                        details,
                    }),
                )))
            }

            ThreadEvent::ItemCompleted(ItemCompletedEvent { item }) => {
                let (item_type, details) = self.map_thread_item(&item)?;
                let item_id = item.id.clone();

                // Remove from state
                self.item_states.remove(&item_id);

                Ok(Some(NormalizedEvent::new(
                    thread_id,
                    self.current_turn_id.clone(),
                    Some(item_id),
                    timestamp,
                    EventSource::ExecJson,
                    "item.completed".to_string(),
                    EventPayload::ItemCompleted(ItemCompletedPayload {
                        item_type,
                        details,
                    }),
                )))
            }

            ThreadEvent::TurnCompleted(TurnCompletedEvent { usage }) => {
                Ok(Some(NormalizedEvent::new(
                    thread_id,
                    self.current_turn_id.clone(),
                    None,
                    timestamp,
                    EventSource::ExecJson,
                    "turn.completed".to_string(),
                    EventPayload::TurnCompleted(TurnCompletedPayload {
                        usage: TokenUsage {
                            input_tokens: usage.input_tokens,
                            cached_input_tokens: usage.cached_input_tokens,
                            output_tokens: usage.output_tokens,
                        },
                    }),
                )))
            }

            ThreadEvent::TurnFailed(TurnFailedEvent { error }) => {
                Ok(Some(NormalizedEvent::new(
                    thread_id,
                    self.current_turn_id.clone(),
                    None,
                    timestamp,
                    EventSource::ExecJson,
                    "turn.failed".to_string(),
                    EventPayload::TurnFailed(TurnFailedPayload {
                        error_message: error.message,
                    }),
                )))
            }

            ThreadEvent::Error(ThreadErrorEvent { message, code }) => {
                Ok(Some(NormalizedEvent::new(
                    thread_id,
                    self.current_turn_id.clone(),
                    None,
                    timestamp,
                    EventSource::ExecJson,
                    "error".to_string(),
                    EventPayload::Error(ErrorPayload { message, code }),
                )))
            }
        }
    }

    fn map_thread_item(&self, item: &ThreadItem) -> Result<(ItemType, ItemDetails), AdapterError> {
        match &item.details {
            ThreadItemDetails::AgentMessage(AgentMessageItem { text, .. }) => {
                Ok((
                    ItemType::AgentMessage,
                    ItemDetails::AgentMessage(AgentMessageDetails {
                        text: text.clone(),
                    }),
                ))
            }

            ThreadItemDetails::Reasoning(ReasoningItem { text }) => {
                Ok((
                    ItemType::Reasoning,
                    ItemDetails::Reasoning(ReasoningDetails {
                        text: text.join("\n"),
                    }),
                ))
            }

            ThreadItemDetails::CommandExecution(CommandExecutionItem {
                command,
                status,
                exit_code,
                aggregated_output,
                ..
            }) => {
                Ok((
                    ItemType::CommandExecution,
                    ItemDetails::CommandExecution(CommandExecutionDetails {
                        command: command.clone(),
                        status: self.map_command_status(status),
                        exit_code: *exit_code,
                        aggregated_output: aggregated_output.clone().unwrap_or_default(),
                    }),
                ))
            }

            ThreadItemDetails::FileChange(file_change) => {
                let changes = file_change
                    .changes
                    .iter()
                    .map(|c| FileChange {
                        path: c.path.to_string_lossy().to_string(),
                        kind: self.map_change_kind(&c.kind),
                    })
                    .collect();

                Ok((
                    ItemType::FileChange,
                    ItemDetails::FileChange(FileChangeDetails {
                        changes,
                        status: self.map_patch_status(&file_change.status),
                    }),
                ))
            }

            ThreadItemDetails::McpToolCall(McpToolCallItem {
                server,
                tool,
                arguments,
                result,
                error,
                status,
                ..
            }) => {
                Ok((
                    ItemType::McpToolCall,
                    ItemDetails::McpToolCall(McpToolCallDetails {
                        server: server.clone(),
                        tool: tool.clone(),
                        arguments: arguments.clone(),
                        result: result.as_ref().map(|r| McpResult {
                            content: r.content.iter().map(|_| serde_json::Value::Null).collect(),
                            structured_content: None,
                        }),
                        error: error.as_ref().map(|e| McpError {
                            message: e.message.clone(),
                        }),
                        status: self.map_mcp_status(status),
                    }),
                ))
            }

            ThreadItemDetails::CollabToolCall(CollabToolCallItem {
                tool,
                sender_thread_id,
                receiver_thread_ids,
                prompt,
                agents_states,
                status,
                ..
            }) => {
                let mapped_states = agents_states
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            CollabAgentState {
                                status: self.map_collab_agent_status(&v.status),
                                message: v.message.clone(),
                            },
                        )
                    })
                    .collect();

                Ok((
                    ItemType::CollabToolCall,
                    ItemDetails::CollabToolCall(CollabToolCallDetails {
                        tool: self.map_collab_tool(tool),
                        sender_thread_id: sender_thread_id.to_string(),
                        receiver_thread_ids: receiver_thread_ids
                            .iter()
                            .map(|id| id.to_string())
                            .collect(),
                        prompt: prompt.clone(),
                        agents_states: mapped_states,
                        status: self.map_collab_status(status),
                    }),
                ))
            }

            ThreadItemDetails::WebSearch(WebSearchItem { id, query, action }) => {
                Ok((
                    ItemType::WebSearch,
                    ItemDetails::WebSearch(WebSearchDetails {
                        query: query.clone(),
                        action: action.as_ref().map(|a| format!("{:?}", a)),
                    }),
                ))
            }

            ThreadItemDetails::TodoList(todo_list) => {
                let items = todo_list
                    .items
                    .iter()
                    .map(|item| TodoItem {
                        text: item.item.clone(),
                        completed: item.status == codex_exec::exec_events::TodoStatus::Completed,
                    })
                    .collect();

                Ok((
                    ItemType::TodoList,
                    ItemDetails::TodoList(TodoListDetails { items }),
                ))
            }

            ThreadItemDetails::Error(ErrorItem { message }) => {
                Ok((
                    ItemType::Error,
                    ItemDetails::Error(ErrorDetails {
                        message: message.clone(),
                    }),
                ))
            }
        }
    }

    fn map_command_status(&self, status: &CommandExecutionStatus) -> ExecutionStatus {
        match status {
            CommandExecutionStatus::InProgress => ExecutionStatus::InProgress,
            CommandExecutionStatus::Completed => ExecutionStatus::Completed,
            CommandExecutionStatus::Failed => ExecutionStatus::Failed,
            CommandExecutionStatus::Declined => ExecutionStatus::Declined,
        }
    }

    fn map_patch_status(&self, status: &PatchApplyStatus) -> PatchStatus {
        match status {
            PatchApplyStatus::InProgress => PatchStatus::InProgress,
            PatchApplyStatus::Completed => PatchStatus::Completed,
            PatchApplyStatus::Failed => PatchStatus::Failed,
        }
    }

    fn map_change_kind(&self, kind: &PatchChangeKind) -> ChangeKind {
        match kind {
            PatchChangeKind::Add => ChangeKind::Add,
            PatchChangeKind::Delete => ChangeKind::Delete,
            PatchChangeKind::Update => ChangeKind::Update,
        }
    }

    fn map_mcp_status(&self, status: &McpToolCallStatus) -> McpStatus {
        match status {
            McpToolCallStatus::InProgress => McpStatus::InProgress,
            McpToolCallStatus::Completed => McpStatus::Completed,
            McpToolCallStatus::Failed => McpStatus::Failed,
        }
    }

    fn map_collab_status(&self, status: &CollabToolCallStatus) -> CollabStatus {
        match status {
            CollabToolCallStatus::InProgress => CollabStatus::InProgress,
            CollabToolCallStatus::Completed => CollabStatus::Completed,
            CollabToolCallStatus::Failed => CollabStatus::Failed,
        }
    }

    fn map_collab_agent_status(&self, status: &codex_exec::exec_events::CollabAgentStatus) -> CollabAgentStatus {
        match status {
            codex_exec::exec_events::CollabAgentStatus::PendingInit => CollabAgentStatus::PendingInit,
            codex_exec::exec_events::CollabAgentStatus::Running => CollabAgentStatus::Running,
            codex_exec::exec_events::CollabAgentStatus::Completed => CollabAgentStatus::Completed,
            codex_exec::exec_events::CollabAgentStatus::Errored => CollabAgentStatus::Errored,
            codex_exec::exec_events::CollabAgentStatus::Shutdown => CollabAgentStatus::Shutdown,
            codex_exec::exec_events::CollabAgentStatus::NotFound => CollabAgentStatus::NotFound,
        }
    }

    fn map_collab_tool(&self, tool: &codex_exec::exec_events::CollabTool) -> CollabTool {
        match tool {
            codex_exec::exec_events::CollabTool::SpawnAgent => CollabTool::SpawnAgent,
            codex_exec::exec_events::CollabTool::SendInput => CollabTool::SendInput,
            codex_exec::exec_events::CollabTool::Wait => CollabTool::Wait,
            codex_exec::exec_events::CollabTool::CloseAgent => CollabTool::CloseAgent,
        }
    }
}

impl<R: AsyncBufRead + Unpin> Stream for ExecJsonAdapter<R> {
    type Item = Result<NormalizedEvent, AdapterError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Check for pending events first
        if let Some(event) = self.pending_events.pop() {
            return Poll::Ready(Some(Ok(event)));
        }

        // Read next line
        self.buffer.clear();
        match futures::ready!(Pin::new(&mut self.reader).poll_read_line(cx, &mut self.buffer)) {
            Ok(0) => Poll::Ready(None), // EOF
            Ok(_) => {
                let line = self.buffer.trim();
                if line.is_empty() {
                    // Empty line, try next
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }

                match serde_json::from_str::<ThreadEvent>(line) {
                    Ok(event) => {
                        match self.process_event(event) {
                            Ok(Some(normalized)) => Poll::Ready(Some(Ok(normalized))),
                            Ok(None) => {
                                // No event generated, continue
                                cx.waker().wake_by_ref();
                                Poll::Pending
                            }
                            Err(e) => {
                                warn!("Error processing event: {}", e);
                                Poll::Ready(Some(Err(e)))
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to parse event: {} - line: {}", e, line);
                        Poll::Ready(Some(Err(AdapterError::ParseError {
                            message: e.to_string(),
                            raw_input: line.to_string(),
                        })))
                    }
                }
            }
            Err(e) => Poll::Ready(Some(Err(AdapterError::IoError(e)))),
        }
    }
}

impl<R: AsyncBufRead + Unpin> EventAdapter for ExecJsonAdapter<R> {
    fn source(&self) -> EventSource {
        EventSource::ExecJson
    }
}
