#![allow(clippy::panic)]

use codex_agentcanvas::EventPayload;
use codex_agentcanvas::EventSource;
use codex_agentcanvas::ItemCompletedPayload;
use codex_agentcanvas::ItemDetails;
use codex_agentcanvas::ItemType;
use codex_agentcanvas::LlmSummaryConfig;
use codex_agentcanvas::NormalizedEvent;
use codex_agentcanvas::SessionSummarizer;
use codex_agentcanvas::SummaryNodeType;
use codex_agentcanvas::TurnStartedPayload;

/// Helper: build minimal events for a single turn.
fn single_turn_events() -> Vec<NormalizedEvent> {
    vec![
        NormalizedEvent::new(
            "thread-llm".to_string(),
            Some("turn-1".to_string()),
            None,
            100,
            EventSource::ExecJson,
            "turn.started".to_string(),
            EventPayload::TurnStarted(TurnStartedPayload {}),
        ),
        NormalizedEvent::new(
            "thread-llm".to_string(),
            Some("turn-1".to_string()),
            Some("msg-1".to_string()),
            101,
            EventSource::ExecJson,
            "item.completed".to_string(),
            EventPayload::ItemCompleted(ItemCompletedPayload {
                item_type: ItemType::AgentMessage,
                details: ItemDetails::AgentMessage(codex_agentcanvas::AgentMessageDetails {
                    text: "Refactored the parser module.".to_string(),
                }),
            }),
        ),
        NormalizedEvent::new(
            "thread-llm".to_string(),
            Some("turn-1".to_string()),
            None,
            102,
            EventSource::ExecJson,
            "turn.completed".to_string(),
            EventPayload::TurnCompleted(codex_agentcanvas::TurnCompletedPayload {
                usage: codex_agentcanvas::TokenUsage {
                    input_tokens: 100,
                    output_tokens: 50,
                    cached_input_tokens: 0,
                },
            }),
        ),
    ]
}

#[tokio::test]
async fn build_with_llm_falls_back_when_no_api_key() {
    // Ensure OPENAI_API_KEY is not set (or empty) for this test.
    // The generate_turn_summaries function checks std::env::var and
    // returns MissingApiKey, causing build_with_llm to fall back.
    //
    // SAFETY: This test is not run in parallel with other tests that depend
    // on OPENAI_API_KEY, so mutating the env is acceptable here.
    let saved = std::env::var("OPENAI_API_KEY").ok();
    unsafe {
        std::env::remove_var("OPENAI_API_KEY");
    }

    let events = single_turn_events();

    let config = LlmSummaryConfig::default();
    let summary = SessionSummarizer::summarize_with_llm(events.iter(), &config)
        .await
        .expect("should produce a summary even without API key");

    // The Turn node should still have the mechanical summary (last agent message).
    let turn_node = summary
        .nodes
        .iter()
        .find(|n| n.node_type == SummaryNodeType::Turn)
        .expect("should have a Turn node");

    assert_eq!(
        turn_node.summary.as_deref(),
        Some("Refactored the parser module.")
    );

    // Restore env if it was set.
    if let Some(key) = saved {
        unsafe {
            std::env::set_var("OPENAI_API_KEY", key);
        }
    }
}

#[tokio::test]
async fn extract_turn_evidence_contains_correct_data() {
    let events = single_turn_events();

    let mut summarizer = SessionSummarizer::new();
    for event in &events {
        summarizer.ingest(event);
    }

    let evidence = summarizer.extract_turn_evidence();
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].turn_id, "turn-1");
    assert_eq!(evidence[0].status.as_deref(), Some("completed"));
    assert_eq!(
        evidence[0].last_agent_message.as_deref(),
        Some("Refactored the parser module.")
    );
}

/// Live test that actually calls the OpenAI API. Requires OPENAI_API_KEY.
/// Run with: cargo test -p codex-agentcanvas -- --ignored live_llm_summary
#[tokio::test]
#[ignore]
async fn live_llm_summary() {
    let events = single_turn_events();

    let config = LlmSummaryConfig::default();
    let summary = SessionSummarizer::summarize_with_llm(events.iter(), &config)
        .await
        .expect("should produce a summary");

    let turn_node = summary
        .nodes
        .iter()
        .find(|n| n.node_type == SummaryNodeType::Turn)
        .expect("should have a Turn node");

    // The LLM should have replaced the mechanical summary with something
    // that is *not* the raw agent message.
    let llm_summary = turn_node
        .summary
        .as_deref()
        .expect("turn should have a summary");

    println!("LLM summary: {llm_summary}");

    // Basic sanity: the summary should be non-empty.
    assert!(!llm_summary.is_empty(), "LLM summary should be non-empty");
}
