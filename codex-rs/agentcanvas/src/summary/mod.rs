//! Session summary schema and deterministic summary builder.

pub(crate) mod llm_summarizer;
mod schema;
mod summarizer;

pub use llm_summarizer::LlmSummaryConfig;
pub use schema::CommandEvidence;
pub use schema::EventReference;
pub use schema::SESSION_SUMMARY_SCHEMA_VERSION;
pub use schema::SessionSummary;
pub use schema::SessionSummaryMetadata;
pub use schema::SummaryEvidence;
pub use schema::SummaryNode;
pub use schema::SummaryNodeType;
pub use schema::TurnLineage;
pub use summarizer::SessionSummarizer;
