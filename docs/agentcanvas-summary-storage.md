# AgentCanvas Summary Storage and Query API (Engineer 3)

`codex-rs/state` now includes a summary storage/index layer intended for
AgentCanvas session summaries.

## Persistence layout

- Summary JSON artifact files are written under:
  - `<codex_home>/agentcanvas/summaries/<thread_id>/<session_id>.json`
- Path components are sanitized to `[A-Za-z0-9._-]` with unsupported characters
  replaced by `_`.
- Summary metadata and node/evidence indexes are persisted in state SQLite via:
  - `summary_artifacts`
  - `summary_nodes`
  - `summary_node_file_paths`
  - `summary_node_commands`
  - `summary_node_errors`

## Runtime API

`StateRuntime` now exposes:

- `upsert_session_summary(...)`
  - Persist JSON artifact + rebuild index rows for the summary.
  - Deterministic default `summary_id` is `<thread_id>:<session_id>`.
- Retrieval by thread/session:
  - `get_session_summary_by_thread_and_session(...)`
  - `get_latest_session_summary_by_thread(...)`
  - `get_latest_session_summary_by_session(...)`
  - `read_session_summary_by_thread_and_session(...)`
  - `read_latest_session_summary_by_thread(...)`
  - `read_latest_session_summary_by_session(...)`
- Node lookup/search:
  - `list_summary_nodes_by_thread_and_session(...)`
  - `search_summary_nodes_by_file_path(...)`
  - `search_summary_nodes_by_command_substring(...)`
  - `search_summary_nodes_by_error_text(...)`

## Core integration

- `codex-rs/core/src/rollout/recorder.rs` now integrates summary persistence in
  the rollout writer path.
- As rollout items stream in, a per-thread turn accumulator tracks command/file/error
  evidence from persisted rollout items.
- On `TurnComplete` or `TurnAborted`, core calls
  `StateRuntime::upsert_session_summary(...)` with:
  - `thread_id` = session thread id
  - `session_id` = turn id
  - deterministic summary id = `agentcanvas.turn:<thread_id>:<turn_id>`

All query methods return indexed node matches with:

- summary identity (`summary_id`, `thread_id`, `session_id`)
- location context (`summary_path`)
- node context (`node_id`, `parent_node_id`, `node_type`, `title`, `node`)
- matched evidence field when applicable (`matched_file_path`,
  `matched_command`, `matched_error_text`)

## Indexing behavior

- Node discovery is schema-tolerant and walks JSON recursively.
- Node-like objects are recognized by:
  - `node_type` / `nodeType`, or
  - `node_id` / `nodeId`, or
  - `id` plus summary-node structure (`parent_id`, `children`, `nodes`, etc.)
- Evidence indexing is key-based:
  - file path keys: `file`, `files`, `file_path`, `file_paths`, `filepath`,
    `filepaths`, `path`, `paths`
  - command keys: `command`, `commands`, `cmd`
  - error keys: `error`, `errors`, `error_text`, `errortext`, `last_error`,
    `stderr`

## Update semantics

- Re-upserting a summary for the same `summary_id` replaces indexed nodes and
  evidence rows.
- This ensures resumed sessions can update summaries without duplicate node
  identities in the index.
