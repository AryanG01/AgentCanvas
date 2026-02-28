# Storage And Index Notes

## Paths
- SQLite DB default resolution in the script:
  1. `--db`
  2. latest numeric `state_<N>.sqlite` under `--codex-home` / `CODEX_SQLITE_HOME` / `CODEX_HOME` / `~/.codex`
  3. `state.sqlite` legacy fallback
  4. `state_<N>.sqlite` where `N` is `CODEX_STATE_DB_VERSION` (if set)
- Summary JSON artifacts are written under:
  - `<codex_home>/agentcanvas/summaries/<thread_id>/<session_id>.json`

## Tables Used For Search
- `summary_artifacts`
- `summary_nodes`
- `summary_node_semantic_terms`
- `summary_node_file_paths`
- `summary_node_commands`
- `summary_node_errors`

## Scoring Model
The script mirrors `codex-rs/state/src/runtime/summaries.rs` for sparse hybrid ranking:
- Semantic score: cosine-style sparse match from `summary_node_semantic_terms`.
- Lexical score weights:
  - file path match: `0.8`
  - command match: `1.0`
  - error text match: `1.2`
  - title match: `0.6`
- Hybrid score: `0.7 * normalized_semantic + 0.3 * normalized_lexical`.

## Practical Notes
- Dense embedding search (Jina API) is not required by this script.
- If dense embeddings exist in DB, they are ignored; sparse + lexical still works offline.
- Returned fields are enough to jump into summary JSON and reconstruct prior context.
