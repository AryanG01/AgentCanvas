use async_openai::Client;
use async_openai::config::OpenAIConfig;
use async_openai::types::chat::ChatCompletionRequestMessage;
use async_openai::types::chat::ChatCompletionRequestSystemMessage;
use async_openai::types::chat::ChatCompletionRequestSystemMessageContent;
use async_openai::types::chat::ChatCompletionRequestUserMessage;
use async_openai::types::chat::ChatCompletionRequestUserMessageContent;
use async_openai::types::chat::CreateChatCompletionRequestArgs;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;
use tokio::time::Duration;

use std::collections::BTreeMap;

pub(crate) const TURN_SUMMARY_LLM_MODEL: &str = "gpt-4.1-nano";
const TURN_SUMMARY_BASE_MAX_TOKENS: u16 = 512;
const TURN_SUMMARY_PER_TURN_TOKENS: u16 = 96;
const TURN_SUMMARY_HARD_MAX_TOKENS: u16 = 2_048;
const TURN_SUMMARY_LLM_TIMEOUT_SECS: u64 = 15;

const SUMMARY_SYSTEM_PROMPT: &str = "\
You summarize coding-agent turns into action-oriented prose.
Lead with the primary action verb: Edited, Ran, Fixed, Searched, Debugged, Read, Created, Deleted, Installed, Built, Tested.
Include specific files/commands when they are the point of the turn.
Mention errors prominently if they occurred.
Return a JSON array where each item has:
  \"turn_id\": the turn identifier,
  \"short_summary\": action-oriented label, max 80 chars (for graph node display),
  \"detail_summary\": expanded detail, max 250 chars (for evidence panel).
Examples:
  short_summary: \"Explored project structure via pwd, ls, find\"
  detail_summary: \"Ran 4 commands to locate portfolio project. Used find with rg to search directories.\"
  short_summary: \"Edited 2 test files, fixed assertion\"
  detail_summary: \"Fixed failing test in src/lib.test.ts by updating expected value.\"
Do not invent facts. Prioritize errors and failures over status text.
Return ONLY JSON, no markdown fences, no extra text.";

#[derive(Debug, Clone, Serialize)]
pub(crate) struct TurnSummarySnapshot {
    pub(crate) turn_id: String,
    pub(crate) status: String,
    pub(crate) signal: String,
    pub(crate) summary_hint: Option<String>,
    pub(crate) primary_command: Option<String>,
    pub(crate) primary_file_path: Option<String>,
    pub(crate) primary_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct TurnSummaryEvidence {
    pub(crate) turn_id: String,
    pub(crate) status: String,
    pub(crate) parent_turn_id: Option<String>,
    pub(crate) child_turn_id: Option<String>,
    pub(crate) forked_from_thread_id: Option<String>,
    pub(crate) started_after_rollback: bool,
    pub(crate) signal: String,
    pub(crate) last_agent_message: Option<String>,
    pub(crate) primary_command: Option<String>,
    pub(crate) primary_file_path: Option<String>,
    pub(crate) primary_error: Option<String>,
    pub(crate) total_commands: usize,
    pub(crate) total_file_paths: usize,
    pub(crate) total_errors: usize,
    pub(crate) command_examples: Vec<String>,
    pub(crate) file_path_examples: Vec<String>,
    pub(crate) error_examples: Vec<String>,
    pub(crate) parent_turn_snapshot: Option<TurnSummarySnapshot>,
    pub(crate) child_turn_snapshot: Option<TurnSummarySnapshot>,
}

#[derive(Debug, Error)]
pub(crate) enum TurnSummaryLlmError {
    #[error("OpenAI API error: {0}")]
    Api(#[from] async_openai::error::OpenAIError),
    #[error("LLM call timed out after {0:?}")]
    Timeout(Duration),
    #[error("failed to parse LLM response as JSON: {0}")]
    Parse(String),
    #[error("no OPENAI_API_KEY set")]
    MissingApiKey,
}

#[derive(Debug, Deserialize)]
struct SummaryItem {
    turn_id: String,
    #[serde(default)]
    short_summary: Option<String>,
    #[serde(default)]
    detail_summary: Option<String>,
    /// Legacy field — used as fallback when short/detail not present.
    #[serde(default)]
    summary: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct LlmSummaryResult {
    /// Short action-oriented label (max ~80 chars) for graph node display.
    pub(crate) short_summary: String,
    /// Expanded detail (max ~250 chars) for evidence panel.
    pub(crate) detail_summary: String,
}

pub(crate) fn build_turn_summary_prompt(turns: &[TurnSummaryEvidence]) -> String {
    serde_json::to_string_pretty(turns).unwrap_or_else(|_| "[]".to_string())
}

pub(crate) async fn generate_turn_summaries(
    turns: &[TurnSummaryEvidence],
) -> Result<BTreeMap<String, LlmSummaryResult>, TurnSummaryLlmError> {
    if turns.is_empty() {
        return Ok(BTreeMap::new());
    }
    if std::env::var("OPENAI_API_KEY")
        .ok()
        .as_deref()
        .is_none_or(str::is_empty)
    {
        return Err(TurnSummaryLlmError::MissingApiKey);
    }

    let completion_tokens = calculate_completion_token_budget(turns.len());

    let request = CreateChatCompletionRequestArgs::default()
        .model(TURN_SUMMARY_LLM_MODEL)
        .max_completion_tokens(u32::from(completion_tokens))
        .temperature(0.3)
        .messages(vec![
            ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage {
                content: ChatCompletionRequestSystemMessageContent::Text(
                    SUMMARY_SYSTEM_PROMPT.to_string(),
                ),
                name: None,
            }),
            ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                content: ChatCompletionRequestUserMessageContent::Text(build_turn_summary_prompt(
                    turns,
                )),
                name: None,
            }),
        ])
        .build()
        .map_err(TurnSummaryLlmError::Api)?;

    let client = Client::with_config(OpenAIConfig::default());
    let response = tokio::time::timeout(
        Duration::from_secs(TURN_SUMMARY_LLM_TIMEOUT_SECS),
        client.chat().create(request),
    )
    .await
    .map_err(|_| TurnSummaryLlmError::Timeout(Duration::from_secs(TURN_SUMMARY_LLM_TIMEOUT_SECS)))?
    .map_err(TurnSummaryLlmError::Api)?;

    let raw = response
        .choices
        .first()
        .and_then(|choice| choice.message.content.as_deref())
        .unwrap_or("[]");
    let items = parse_summary_items(raw).map_err(TurnSummaryLlmError::Parse)?;
    let mut map = BTreeMap::new();
    for item in items {
        let short = item
            .short_summary
            .or(item.summary.clone())
            .unwrap_or_default();
        let detail = item
            .detail_summary
            .or(item.summary)
            .unwrap_or_default();
        map.insert(
            item.turn_id,
            LlmSummaryResult {
                short_summary: short,
                detail_summary: detail,
            },
        );
    }
    Ok(map)
}

fn calculate_completion_token_budget(turn_count: usize) -> u16 {
    let base_tokens = usize::from(TURN_SUMMARY_BASE_MAX_TOKENS);
    let per_turn_tokens = usize::from(TURN_SUMMARY_PER_TURN_TOKENS);
    let hard_max_tokens = usize::from(TURN_SUMMARY_HARD_MAX_TOKENS);
    let requested = base_tokens.saturating_add(turn_count.saturating_mul(per_turn_tokens));
    let capped = requested.min(hard_max_tokens);
    u16::try_from(capped).unwrap_or(TURN_SUMMARY_HARD_MAX_TOKENS)
}

fn parse_summary_items(raw_response: &str) -> Result<Vec<SummaryItem>, String> {
    let trimmed = raw_response.trim();
    if let Ok(items) = serde_json::from_str::<Vec<SummaryItem>>(trimmed) {
        return Ok(items);
    }
    let array_start = trimmed
        .find('[')
        .ok_or_else(|| "missing JSON array".to_string())?;
    let array_end = trimmed
        .rfind(']')
        .ok_or_else(|| "missing JSON array".to_string())?;
    if array_end <= array_start {
        return Err("invalid JSON array bounds".to_string());
    }
    let array = &trimmed[array_start..=array_end];
    serde_json::from_str::<Vec<SummaryItem>>(array).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn build_turn_summary_prompt_is_valid_json() {
        let evidence = vec![TurnSummaryEvidence {
            turn_id: "turn-1".to_string(),
            status: "completed".to_string(),
            parent_turn_id: None,
            child_turn_id: Some("turn-2".to_string()),
            forked_from_thread_id: Some("thread-x".to_string()),
            started_after_rollback: false,
            signal: "execution".to_string(),
            last_agent_message: Some("Done".to_string()),
            primary_command: Some("rg summary".to_string()),
            primary_file_path: Some("src/main.rs".to_string()),
            primary_error: None,
            total_commands: 1,
            total_file_paths: 2,
            total_errors: 0,
            command_examples: vec!["rg summary".to_string()],
            file_path_examples: vec!["src/main.rs".to_string()],
            error_examples: vec![],
            parent_turn_snapshot: None,
            child_turn_snapshot: Some(TurnSummarySnapshot {
                turn_id: "turn-2".to_string(),
                status: "in_progress".to_string(),
                signal: "status_only".to_string(),
                summary_hint: Some("continue with tests".to_string()),
                primary_command: Some("cargo test -p codex-core".to_string()),
                primary_file_path: None,
                primary_error: None,
            }),
        }];

        let prompt = build_turn_summary_prompt(&evidence);
        let parsed: serde_json::Value =
            serde_json::from_str(&prompt).expect("prompt should parse as valid JSON");
        assert_eq!(parsed.as_array().expect("should be array").len(), 1);
        assert_eq!(parsed[0]["turn_id"], "turn-1");
        assert_eq!(parsed[0]["status"], "completed");
        assert_eq!(
            parsed[0]["child_turn_snapshot"]["turn_id"],
            serde_json::json!("turn-2")
        );
    }

    #[test]
    fn parse_summary_items_handles_wrapped_markdown() {
        let raw = "```json\n[{\"turn_id\":\"turn-1\",\"summary\":\"summary text\"}]\n```";
        let items = parse_summary_items(raw).expect("wrapped output should still parse");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].turn_id, "turn-1");
        assert_eq!(items[0].summary, "summary text");
    }

    #[test]
    fn completion_budget_scales_with_turn_count() {
        assert_eq!(calculate_completion_token_budget(1), 608);
        assert_eq!(
            calculate_completion_token_budget(30),
            TURN_SUMMARY_HARD_MAX_TOKENS
        );
    }
}
