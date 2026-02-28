//! Translates `NormalizedEvent` streams into `AppEvent` types for the UI.

use crate::schema::EventPayload;
use crate::schema::ItemDetails;
use crate::schema::McpStatus;
use crate::schema::NormalizedEvent;
use crate::schema::PatchStatus;

use super::app_event::AppEvent;

/// Translate a single `NormalizedEvent` into zero or more `AppEvent`s.
///
/// Returns a `Vec` because some events (e.g. `FileChange` with multiple files)
/// fan out into multiple UI events.
pub fn translate(event: &NormalizedEvent) -> Vec<AppEvent> {
    let turn_id = event
        .turn_id
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    let item_id = event
        .item_id
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    let ts = event.timestamp;

    match &event.payload {
        EventPayload::ThreadStarted(_) => {
            vec![AppEvent::ThreadStarted {
                thread_id: event.thread_id.clone(),
                ts,
            }]
        }

        EventPayload::TurnStarted(_) => {
            vec![AppEvent::TurnStarted {
                turn_id,
                session_id: event.thread_id.clone(),
                user_prompt: String::new(),
                ts,
            }]
        }

        EventPayload::TurnCompleted(_) => {
            vec![AppEvent::TurnComplete {
                turn_id,
                status: "success".to_string(),
                ts,
            }]
        }

        EventPayload::TurnFailed(_) => {
            vec![AppEvent::TurnComplete {
                turn_id,
                status: "error".to_string(),
                ts,
            }]
        }

        EventPayload::ItemCompleted(payload) => match &payload.details {
            ItemDetails::CommandExecution(details) => {
                vec![AppEvent::CommandExecution {
                    id: item_id,
                    turn_id,
                    cmd: details.command.clone(),
                    exit_code: details.exit_code,
                    stdout: details.aggregated_output.clone(),
                    stderr: String::new(),
                    ts,
                }]
            }

            ItemDetails::McpToolCall(details) => {
                let output = details.result.as_ref().map(|r| {
                    r.structured_content
                        .clone()
                        .unwrap_or_else(|| serde_json::Value::Array(r.content.clone()))
                });
                let status = match details.status {
                    McpStatus::Completed => "success",
                    _ => "error",
                };
                vec![AppEvent::McpToolCall {
                    id: item_id,
                    turn_id,
                    tool_name: format!("{}/{}", details.server, details.tool),
                    input: details.arguments.clone(),
                    output,
                    status: status.to_string(),
                    ts,
                }]
            }

            ItemDetails::FileChange(details) => {
                let status = match details.status {
                    PatchStatus::Completed => "success",
                    _ => "error",
                };
                details
                    .changes
                    .iter()
                    .enumerate()
                    .map(|(i, change)| {
                        let patch_id = if details.changes.len() == 1 {
                            item_id.clone()
                        } else {
                            format!("{item_id}:{i}")
                        };
                        AppEvent::PatchApply {
                            id: patch_id,
                            turn_id: turn_id.clone(),
                            filename: change.path.clone(),
                            diff: format!("{:?}", change.kind).to_lowercase(),
                            status: status.to_string(),
                            ts,
                        }
                    })
                    .collect()
            }

            ItemDetails::TodoList(details) => {
                let text = details
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
                    .join("\n");
                vec![AppEvent::PlanUpdate {
                    id: item_id,
                    turn_id,
                    text,
                    ts,
                }]
            }

            // Other item types (AgentMessage, Reasoning, WebSearch, CollabToolCall, Error)
            // don't have a direct AppEvent mapping for the UI.
            _ => vec![],
        },

        // Streaming and close events don't produce UI AppEvents.
        EventPayload::ThreadClosed
        | EventPayload::ItemStarted(_)
        | EventPayload::ItemUpdated(_)
        | EventPayload::ItemDelta(_)
        | EventPayload::Error(_) => vec![],
    }
}
