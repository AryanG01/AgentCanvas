# codex-agentcanvas

Event normalization layer for AgentCanvas observability system.

## Overview

This crate provides a unified normalization layer that transforms events from three heterogeneous sources into a single canonical `NormalizedEvent` representation:

1. **exec --experimental-json**: JSONL stream from CLI execution (`codex-rs/exec`)
2. **app-server v2 notifications**: JSON-RPC 2.0 over stdio/WebSocket (`codex-rs/app-server`)
3. **persisted rollout files**: JSONL session history files from `~/.codex/sessions/`

All events are normalized to enable downstream summarization, storage, and querying.

## Architecture

### Schema (`src/schema/`)

The normalized event schema consists of:

- **NormalizedEvent**: Top-level envelope with universal metadata
  - `thread_id`, `turn_id`, `item_id`: Hierarchical identifiers
  - `timestamp`: Unix seconds
  - `schema_version`: Explicit versioning (currently v1)
  - `source`: Provenance tracking (ExecJson | AppServerV2 | RolloutReplay)
  - `source_event_type`: Original event type for debugging
  - `payload`: Event-specific data

- **EventPayload**: Discriminated union of lifecycle events
  - Thread: `ThreadStarted`, `ThreadClosed`
  - Turn: `TurnStarted`, `TurnCompleted`, `TurnFailed`
  - Item: `ItemStarted`, `ItemUpdated`, `ItemCompleted`
  - Streaming: `ItemDelta` (app-server only)
  - Error: `Error`

- **ItemDetails**: 9 item types with specific fields
  - `AgentMessage`, `Reasoning`, `CommandExecution`, `FileChange`
  - `McpToolCall`, `CollabToolCall`, `WebSearch`, `TodoList`, `Error`

### Adapters (`src/adapters/`)

Each adapter implements the `EventAdapter` trait (async `Stream<Item = Result<NormalizedEvent, AdapterError>>`):

- **ExecJsonAdapter**: Parses `ThreadEvent` JSONL stream from `exec --experimental-json`
  - Maintains state to match begin/end events
  - Generates synthetic turn IDs (`turn_0`, `turn_1`, ...)
  - Maps all item types to normalized schema

- **AppServerV2Adapter**: Parses JSON-RPC 2.0 notifications from app-server
  - Handles camelCase → snake_case field mapping
  - Emits both deltas (streaming) and snapshots (ItemUpdated)
  - Preserves turn_id and item_id from notifications

- **RolloutReplayAdapter**: Replays persisted JSONL rollout files
  - Generates synthetic turn_id and item_id (many events lack these)
  - Parses `RolloutLine` → `RolloutItem` → `EventMsg`
  - Deterministic ID generation for reproducibility

## Usage

### Basic Example

```rust
use codex_agentcanvas::{ExecJsonAdapter, EventAdapter, NormalizedEvent};
use futures::StreamExt;
use tokio::fs::File;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Open exec JSON stream
    let file = File::open("session.jsonl").await?;
    let adapter = ExecJsonAdapter::new(file);

    // Consume normalized events
    let mut stream = Box::pin(adapter);
    while let Some(result) = stream.next().await {
        match result {
            Ok(event) => println!("Event: {:?}", event),
            Err(e) => eprintln!("Error: {}", e),
        }
    }

    Ok(())
}
```

### Adapter Selection

```rust
use codex_agentcanvas::{EventSource, ExecJsonAdapter, AppServerV2Adapter, RolloutReplayAdapter};

// Choose adapter based on source
let adapter: Box<dyn EventAdapter> = match source {
    EventSource::ExecJson => Box::new(ExecJsonAdapter::new(reader)),
    EventSource::AppServerV2 => Box::new(AppServerV2Adapter::new(reader)),
    EventSource::RolloutReplay => Box::new(RolloutReplayAdapter::new(reader)),
};
```

## Design Principles

1. **Determinism**: Same input → same output (critical for reproducibility)
2. **Explicit Versioning**: `schema_version` field enables forward compatibility
3. **Provenance Tracking**: `source` and `source_event_type` for debugging
4. **Stateful Adapters**: Match begin/end events internally for clean API
5. **No Deduplication**: Pure transformation; deduplication happens downstream

## Schema Versioning

Current schema version: **1**

Breaking changes will increment `schema_version`. Downstream consumers should check `event.schema_version` and handle migrations.

## Testing

Run tests with:

```bash
cargo test -p codex-agentcanvas
```

Equivalence tests verify that the same logical turn produces equivalent normalized events across all three adapters.

## Dependencies

- `codex-exec`: Exec event types
- `codex-app-server-protocol`: App-server v2 notifications
- `codex-protocol`: Rollout types
- `tokio`, `futures`: Async streaming
- `serde`, `serde_json`: Serialization
- `ts-rs`: TypeScript type generation

## Future Work

- [ ] Complete `AppServerV2Adapter` implementation (delta handling)
- [ ] Complete `RolloutReplayAdapter` implementation (all EventMsg variants)
- [ ] Add comprehensive test fixtures
- [ ] Implement equivalence test helpers
- [ ] Add snapshot testing with `insta`

## Related Crates

This crate is part of the AgentCanvas observability system:

- **Engineer 1 (this crate)**: Event ingestion + normalization
- **Engineer 2**: Summary engine + JSON schema
- **Engineer 3**: Summary storage + query/retrieval API
- **Engineer 4**: Realtime graph UI + interaction

See `docs/agentcanvas-agents.md` for the full collaboration plan.
