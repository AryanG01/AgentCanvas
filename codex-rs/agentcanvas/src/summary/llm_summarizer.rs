//! LLM-based turn summarization via OpenAI API.
//!
//! After the mechanical `SessionSummary` is built, this module can replace
//! each Turn node's `summary` field with a 1–2 sentence natural-language
//! description produced by an LLM (default: `gpt-4.1-nano`).
//!
//! On any failure (missing API key, timeout, bad JSON, network error) the
//! caller falls back to the original mechanical summaries.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

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

// ── Configuration ────────────────────────────────────────────────────

/// Configuration for the LLM summarization pass.
#[derive(Debug, Clone)]
pub struct LlmSummaryConfig {
    /// OpenAI model id (default: `gpt-4.1-nano`).
    pub model: String,
    /// Maximum tokens for the completion (default: 1024).
    pub max_tokens: u16,
    /// Sampling temperature (default: 0.3).
    pub temperature: f32,
}

impl Default for LlmSummaryConfig {
    fn default() -> Self {
        Self {
            model: "gpt-4.1-nano".to_string(),
            max_tokens: 1024,
            temperature: 0.3,
        }
    }
}

// ── Evidence extracted from TurnAccumulator ──────────────────────────

/// A read-only view of the evidence accumulated during a single turn,
/// suitable for serialization into an LLM prompt.
#[derive(Debug, Clone, Serialize)]
pub struct TurnEvidence {
    pub turn_id: String,
    pub status: Option<String>,
    pub last_agent_message: Option<String>,
    pub plan_updates: Vec<String>,
    pub commands: Vec<String>,
    pub file_paths: Vec<String>,
    pub external_tools: Vec<String>,
    pub errors: Vec<String>,
}

// ── Errors ───────────────────────────────────────────────────────────

/// Errors that can occur during LLM summarization.
#[derive(Debug, Error)]
pub enum LlmSummaryError {
    #[error("OpenAI API error: {0}")]
    Api(#[from] async_openai::error::OpenAIError),

    #[error("LLM call timed out after {0:?}")]
    Timeout(Duration),

    #[error("failed to parse LLM response as JSON: {0}")]
    Parse(String),

    #[error("no OPENAI_API_KEY set")]
    MissingApiKey,
}

// ── Prompt construction ──────────────────────────────────────────────

const SYSTEM_PROMPT: &str = "\
You summarize coding-agent turns. For each turn you receive structured \
evidence (status, last message, plan updates, commands run, files changed, \
external tools used, and errors). Produce a JSON array where each element is \
{\"turn_id\": \"<id>\", \"summary\": \"<1-2 sentence summary>\"}. \
Return ONLY the JSON array, no markdown fences, no extra text.";

/// Build the user-message content from the turn evidence.
pub(crate) fn build_user_prompt(turns: &[TurnEvidence]) -> String {
    serde_json::to_string_pretty(turns).unwrap_or_else(|_| "[]".to_string())
}

// ── Core API call ────────────────────────────────────────────────────

/// Response item expected from the LLM.
#[derive(Debug, Deserialize)]
struct TurnSummaryItem {
    turn_id: String,
    summary: String,
}

/// Call the OpenAI API to generate natural-language summaries for every
/// turn described in `turns`.
///
/// Returns a map from `turn_id` → summary string.
pub(crate) async fn generate_turn_summaries(
    config: &LlmSummaryConfig,
    turns: &[TurnEvidence],
) -> Result<BTreeMap<String, String>, LlmSummaryError> {
    if turns.is_empty() {
        return Ok(BTreeMap::new());
    }

    // Load .env.local (then .env) so users can store OPENAI_API_KEY there.
    // Errors are silently ignored — the file is optional.
    let _ = dotenvy::from_filename(".env.local");
    let _ = dotenvy::dotenv();

    // Fail fast when there is no API key.
    if std::env::var("OPENAI_API_KEY").unwrap_or_default().is_empty() {
        return Err(LlmSummaryError::MissingApiKey);
    }

    let client = Client::with_config(OpenAIConfig::default());

    let user_content = build_user_prompt(turns);

    let request = CreateChatCompletionRequestArgs::default()
        .model(&config.model)
        .max_completion_tokens(u32::from(config.max_tokens))
        .temperature(config.temperature)
        .messages(vec![
            ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage {
                content: ChatCompletionRequestSystemMessageContent::Text(
                    SYSTEM_PROMPT.to_string(),
                ),
                name: None,
            }),
            ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                content: ChatCompletionRequestUserMessageContent::Text(user_content),
                name: None,
            }),
        ])
        .build()
        .map_err(LlmSummaryError::Api)?;

    let response = tokio::time::timeout(Duration::from_secs(10), client.chat().create(request))
        .await
        .map_err(|_| LlmSummaryError::Timeout(Duration::from_secs(10)))?
        .map_err(LlmSummaryError::Api)?;

    let raw = response
        .choices
        .first()
        .and_then(|c| c.message.content.as_deref())
        .unwrap_or("[]");

    let items: Vec<TurnSummaryItem> =
        serde_json::from_str(raw).map_err(|e| LlmSummaryError::Parse(e.to_string()))?;

    let mut map = BTreeMap::new();
    for item in items {
        map.insert(item.turn_id, item.summary);
    }
    Ok(map)
}

// ── Helpers for extracting evidence from TurnAccumulator ─────────────

/// Extract `TurnEvidence` from the accumulator map. This is called
/// *before* `build()` consumes the summarizer so we can read the data.
pub(crate) fn extract_turn_evidence(
    turns: &BTreeMap<String, super::summarizer::TurnAccumulator>,
) -> Vec<TurnEvidence> {
    turns
        .iter()
        .map(|(turn_id, acc)| TurnEvidence {
            turn_id: turn_id.clone(),
            status: acc.status.clone(),
            last_agent_message: acc.last_agent_message.clone(),
            plan_updates: acc.plan_updates.clone(),
            commands: acc
                .commands
                .iter()
                .map(|c| c.command.clone())
                .collect(),
            file_paths: btree_to_vec(&acc.file_paths),
            external_tools: btree_to_vec(&acc.external_tools),
            errors: btree_to_vec(&acc.errors),
        })
        .collect()
}

fn btree_to_vec(set: &BTreeSet<String>) -> Vec<String> {
    set.iter().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_prompt_formats_correctly() {
        let evidence = vec![TurnEvidence {
            turn_id: "turn-1".to_string(),
            status: Some("completed".to_string()),
            last_agent_message: Some("Done editing files.".to_string()),
            plan_updates: vec!["[x] step 1".to_string()],
            commands: vec!["cargo test".to_string()],
            file_paths: vec!["src/main.rs".to_string()],
            external_tools: vec![],
            errors: vec![],
        }];

        let prompt = build_user_prompt(&evidence);

        // Must be valid JSON.
        let parsed: serde_json::Value =
            serde_json::from_str(&prompt).expect("prompt must be valid JSON");
        assert!(parsed.is_array());

        let arr = parsed.as_array().expect("should be array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["turn_id"], "turn-1");
        assert_eq!(arr[0]["status"], "completed");
        assert_eq!(arr[0]["last_agent_message"], "Done editing files.");

        // System prompt should be non-empty and mention JSON.
        assert!(SYSTEM_PROMPT.contains("JSON"));
    }
}
