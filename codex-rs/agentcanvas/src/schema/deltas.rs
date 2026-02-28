//! Streaming delta types for incremental updates

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::common::OutputStream;

/// Delta details for streaming updates
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "delta_type", rename_all = "snake_case")]
#[ts(export)]
pub enum DeltaDetails {
    AgentMessageDelta {
        text_delta: String,
    },
    CommandOutputDelta {
        output_delta: String,
        stream: OutputStream,
    },
    FileChangeDelta {
        output_delta: String,
    },
    ReasoningDelta {
        text_delta: String,
    },
}
