#![allow(clippy::panic)]

use codex_agentcanvas::CollabAgentState;
use codex_agentcanvas::CollabAgentStatus;
use codex_agentcanvas::CollabStatus;
use codex_agentcanvas::CommandExecutionDetails;
use codex_agentcanvas::EventPayload;
use codex_agentcanvas::EventSource;
use codex_agentcanvas::FileChange;
use codex_agentcanvas::FileChangeDetails;
use codex_agentcanvas::ItemCompletedPayload;
use codex_agentcanvas::ItemDetails;
use codex_agentcanvas::ItemType;
use codex_agentcanvas::McpError;
use codex_agentcanvas::McpStatus;
use codex_agentcanvas::McpToolCallDetails;
use codex_agentcanvas::NormalizedEvent;
use codex_agentcanvas::SessionSummarizer;
use codex_agentcanvas::TodoItem;
use codex_agentcanvas::TodoListDetails;
use codex_agentcanvas::TurnFailedPayload;
use codex_agentcanvas::TurnStartedPayload;
use codex_agentcanvas::{translate_to_app_event, AppEvent};
use pretty_assertions::assert_eq;
use std::collections::HashMap;

// NOTE: Fixture-based snapshot tests (summary_snapshot_for_basic_turn_fixture,
// summary_snapshot_for_command_lifecycle_fixture, summary_snapshot_for_multi_turn_fixture)
// are temporarily disabled because they depend on ExecJsonAdapter which has
// pre-existing compilation errors in the adapters module. Re-enable when adapters are fixed.

#[test]
fn summary_promotes_failures_major_plan_pivots_and_large_edits() {
    let thread_id = "thread-promotion-test";
    let turn_id = "turn-1";

    let events = [
        NormalizedEvent::new(
            thread_id.to_string(),
            Some(turn_id.to_string()),
            None,
            10,
            EventSource::ExecJson,
            "turn.started".to_string(),
            EventPayload::TurnStarted(TurnStartedPayload {}),
        ),
        normalized_item_completed_event(
            thread_id,
            turn_id,
            "plan-1",
            11,
            ItemDetails::TodoList(TodoListDetails {
                items: vec![
                    TodoItem {
                        text: "inspect files".to_string(),
                        completed: true,
                    },
                    TodoItem {
                        text: "update summary".to_string(),
                        completed: false,
                    },
                ],
            }),
        ),
        normalized_item_completed_event(
            thread_id,
            turn_id,
            "plan-2",
            12,
            ItemDetails::TodoList(TodoListDetails {
                items: vec![
                    TodoItem {
                        text: "rework parser".to_string(),
                        completed: true,
                    },
                    TodoItem {
                        text: "add tests".to_string(),
                        completed: false,
                    },
                ],
            }),
        ),
        normalized_item_completed_event(
            thread_id,
            turn_id,
            "cmd-1",
            13,
            ItemDetails::CommandExecution(CommandExecutionDetails {
                command: "cargo test -p codex-agentcanvas".to_string(),
                status: codex_agentcanvas::ExecutionStatus::Failed,
                exit_code: Some(1),
                aggregated_output: "test failure output".to_string(),
            }),
        ),
        normalized_item_completed_event(
            thread_id,
            turn_id,
            "patch-1",
            14,
            ItemDetails::FileChange(FileChangeDetails {
                changes: vec![
                    FileChange {
                        path: "src/a.rs".to_string(),
                        kind: codex_agentcanvas::ChangeKind::Update,
                    },
                    FileChange {
                        path: "src/b.rs".to_string(),
                        kind: codex_agentcanvas::ChangeKind::Update,
                    },
                    FileChange {
                        path: "src/c.rs".to_string(),
                        kind: codex_agentcanvas::ChangeKind::Update,
                    },
                ],
                status: codex_agentcanvas::PatchStatus::Failed,
            }),
        ),
        normalized_item_completed_event(
            thread_id,
            turn_id,
            "tool-1",
            15,
            ItemDetails::McpToolCall(McpToolCallDetails {
                server: "filesystem".to_string(),
                tool: "read_file".to_string(),
                arguments: serde_json::json!({"path": "src/a.rs"}),
                result: None,
                error: Some(McpError {
                    message: "permission denied".to_string(),
                }),
                status: McpStatus::Failed,
            }),
        ),
        normalized_item_completed_event(
            thread_id,
            turn_id,
            "collab-1",
            16,
            ItemDetails::CollabToolCall(codex_agentcanvas::CollabToolCallDetails {
                tool: codex_agentcanvas::CollabTool::SpawnAgent,
                sender_thread_id: thread_id.to_string(),
                receiver_thread_ids: vec!["other-thread".to_string()],
                prompt: Some("help".to_string()),
                agents_states: HashMap::from([(
                    "other-thread".to_string(),
                    CollabAgentState {
                        status: CollabAgentStatus::Errored,
                        message: Some("failed".to_string()),
                    },
                )]),
                status: CollabStatus::Failed,
            }),
        ),
        NormalizedEvent::new(
            thread_id.to_string(),
            Some(turn_id.to_string()),
            None,
            17,
            EventSource::ExecJson,
            "turn.failed".to_string(),
            EventPayload::TurnFailed(TurnFailedPayload {
                error_message: "agent failed".to_string(),
            }),
        ),
    ];

    let Some(summary) = SessionSummarizer::summarize(events.iter()) else {
        panic!("expected summary for synthetic events");
    };

    let promoted_node_types: Vec<String> = summary
        .nodes
        .iter()
        .filter(|node| node.promoted)
        .map(|node| node.node_type.to_string())
        .collect();

    assert_eq!(
        promoted_node_types,
        vec![
            "plan".to_string(),
            "execution".to_string(),
            "code_changes".to_string(),
            "external_tools".to_string(),
            "error".to_string(),
        ]
    );
}

#[test]
fn bridge_translates_normalized_events_to_app_events() {
    let thread_id = "thread-bridge-test";
    let turn_id = "turn-1";

    // TurnStarted -> AppEvent::TurnStarted
    let turn_started = NormalizedEvent::new(
        thread_id.to_string(),
        Some(turn_id.to_string()),
        None,
        100,
        EventSource::ExecJson,
        "turn.started".to_string(),
        EventPayload::TurnStarted(TurnStartedPayload {}),
    );
    let app_events = translate_to_app_event(&turn_started);
    assert_eq!(app_events.len(), 1);
    match &app_events[0] {
        AppEvent::TurnStarted {
            turn_id: tid,
            session_id,
            ts,
        } => {
            assert_eq!(tid, turn_id);
            assert_eq!(session_id, thread_id);
            assert_eq!(*ts, 100);
        }
        other => panic!("expected TurnStarted, got {other:?}"),
    }

    // CommandExecution -> AppEvent::CommandExecution
    let cmd_event = normalized_item_completed_event(
        thread_id,
        turn_id,
        "cmd-1",
        101,
        ItemDetails::CommandExecution(CommandExecutionDetails {
            command: "echo hello".to_string(),
            status: codex_agentcanvas::ExecutionStatus::Completed,
            exit_code: Some(0),
            aggregated_output: "hello\n".to_string(),
        }),
    );
    let app_events = translate_to_app_event(&cmd_event);
    assert_eq!(app_events.len(), 1);
    match &app_events[0] {
        AppEvent::CommandExecution {
            id,
            cmd,
            exit_code,
            stdout,
            ..
        } => {
            assert_eq!(id, "cmd-1");
            assert_eq!(cmd, "echo hello");
            assert_eq!(*exit_code, Some(0));
            assert_eq!(stdout, "hello\n");
        }
        other => panic!("expected CommandExecution, got {other:?}"),
    }

    // McpToolCall -> AppEvent::McpToolCall
    let mcp_event = normalized_item_completed_event(
        thread_id,
        turn_id,
        "tool-1",
        102,
        ItemDetails::McpToolCall(McpToolCallDetails {
            server: "filesystem".to_string(),
            tool: "read_file".to_string(),
            arguments: serde_json::json!({"path": "test.rs"}),
            result: None,
            error: None,
            status: McpStatus::Completed,
        }),
    );
    let app_events = translate_to_app_event(&mcp_event);
    assert_eq!(app_events.len(), 1);
    match &app_events[0] {
        AppEvent::McpToolCall {
            id,
            tool_name,
            status,
            ..
        } => {
            assert_eq!(id, "tool-1");
            assert_eq!(tool_name, "filesystem/read_file");
            assert_eq!(status, "completed");
        }
        other => panic!("expected McpToolCall, got {other:?}"),
    }

    // FileChange with multiple files -> multiple PatchApply events
    let file_event = normalized_item_completed_event(
        thread_id,
        turn_id,
        "patch-1",
        103,
        ItemDetails::FileChange(FileChangeDetails {
            changes: vec![
                FileChange {
                    path: "src/a.rs".to_string(),
                    kind: codex_agentcanvas::ChangeKind::Update,
                },
                FileChange {
                    path: "src/b.rs".to_string(),
                    kind: codex_agentcanvas::ChangeKind::Add,
                },
            ],
            status: codex_agentcanvas::PatchStatus::Completed,
        }),
    );
    let app_events = translate_to_app_event(&file_event);
    assert_eq!(app_events.len(), 2, "multi-file change should fan out");
    match &app_events[0] {
        AppEvent::PatchApply {
            id, filename, diff, ..
        } => {
            assert_eq!(id, "patch-1:0");
            assert_eq!(filename, "src/a.rs");
            assert_eq!(diff, "update");
        }
        other => panic!("expected PatchApply, got {other:?}"),
    }
    match &app_events[1] {
        AppEvent::PatchApply {
            id, filename, diff, ..
        } => {
            assert_eq!(id, "patch-1:1");
            assert_eq!(filename, "src/b.rs");
            assert_eq!(diff, "add");
        }
        other => panic!("expected PatchApply, got {other:?}"),
    }

    // TodoList -> AppEvent::PlanUpdate
    let todo_event = normalized_item_completed_event(
        thread_id,
        turn_id,
        "plan-1",
        104,
        ItemDetails::TodoList(TodoListDetails {
            items: vec![
                TodoItem {
                    text: "step 1".to_string(),
                    completed: true,
                },
                TodoItem {
                    text: "step 2".to_string(),
                    completed: false,
                },
            ],
        }),
    );
    let app_events = translate_to_app_event(&todo_event);
    assert_eq!(app_events.len(), 1);
    match &app_events[0] {
        AppEvent::PlanUpdate { id, text, .. } => {
            assert_eq!(id, "plan-1");
            assert!(text.contains("[x] step 1"));
            assert!(text.contains("[ ] step 2"));
        }
        other => panic!("expected PlanUpdate, got {other:?}"),
    }

    // TurnCompleted -> AppEvent::TurnComplete
    let turn_completed = NormalizedEvent::new(
        thread_id.to_string(),
        Some(turn_id.to_string()),
        None,
        105,
        EventSource::ExecJson,
        "turn.completed".to_string(),
        EventPayload::TurnCompleted(codex_agentcanvas::TurnCompletedPayload {
            usage: codex_agentcanvas::TokenUsage {
                input_tokens: 100,
                cached_input_tokens: 50,
                output_tokens: 25,
            },
        }),
    );
    let app_events = translate_to_app_event(&turn_completed);
    assert_eq!(app_events.len(), 1);
    match &app_events[0] {
        AppEvent::TurnComplete { status, .. } => {
            assert_eq!(status, "completed");
        }
        other => panic!("expected TurnComplete, got {other:?}"),
    }

    // TurnFailed -> AppEvent::TurnComplete with "failed" status
    let turn_failed = NormalizedEvent::new(
        thread_id.to_string(),
        Some(turn_id.to_string()),
        None,
        106,
        EventSource::ExecJson,
        "turn.failed".to_string(),
        EventPayload::TurnFailed(TurnFailedPayload {
            error_message: "something broke".to_string(),
        }),
    );
    let app_events = translate_to_app_event(&turn_failed);
    assert_eq!(app_events.len(), 1);
    match &app_events[0] {
        AppEvent::TurnComplete { status, .. } => {
            assert_eq!(status, "failed");
        }
        other => panic!("expected TurnComplete, got {other:?}"),
    }

    // ThreadStarted -> no AppEvent
    let thread_started = NormalizedEvent::new(
        thread_id.to_string(),
        None,
        None,
        99,
        EventSource::ExecJson,
        "thread.started".to_string(),
        EventPayload::ThreadStarted(codex_agentcanvas::ThreadStartedPayload {}),
    );
    let app_events = translate_to_app_event(&thread_started);
    assert!(
        app_events.is_empty(),
        "thread-level events should not produce AppEvents"
    );
}

fn normalized_item_completed_event(
    thread_id: &str,
    turn_id: &str,
    item_id: &str,
    timestamp: i64,
    details: ItemDetails,
) -> NormalizedEvent {
    let item_type = match &details {
        ItemDetails::AgentMessage(_) => ItemType::AgentMessage,
        ItemDetails::Reasoning(_) => ItemType::Reasoning,
        ItemDetails::CommandExecution(_) => ItemType::CommandExecution,
        ItemDetails::FileChange(_) => ItemType::FileChange,
        ItemDetails::McpToolCall(_) => ItemType::McpToolCall,
        ItemDetails::CollabToolCall(_) => ItemType::CollabToolCall,
        ItemDetails::WebSearch(_) => ItemType::WebSearch,
        ItemDetails::TodoList(_) => ItemType::TodoList,
        ItemDetails::Error(_) => ItemType::Error,
    };

    NormalizedEvent::new(
        thread_id.to_string(),
        Some(turn_id.to_string()),
        Some(item_id.to_string()),
        timestamp,
        EventSource::ExecJson,
        "item.completed".to_string(),
        EventPayload::ItemCompleted(ItemCompletedPayload { item_type, details }),
    )
}
