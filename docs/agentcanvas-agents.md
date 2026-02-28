# AgentCanvas Collaboration Plan (4 Engineers)

## Goal

Build AgentCanvas as a lightweight observability + memory layer on top of Codex CLI/session artifacts:

- Capture key session events (prompt, plan, tool actions, outcomes).
- Convert them into compact, structured summaries.
- Render summaries as an expandable parent->child graph.
- Keep raw transcripts as source-of-truth evidence; keep summaries small and queryable.

## Build Constraints

- Reuse existing Codex event surfaces first:
  - `codex exec --experimental-json` (`codex-rs/exec/src/exec_events.rs`, `sdk/typescript/src/events.ts`).
  - app-server v2 notifications (`codex-rs/app-server/README.md`, `codex-rs/app-server-protocol/src/protocol/v2.rs`).
  - persisted thread/session rollouts via existing session storage APIs.
- Keep backend changes in Rust crates under `codex-rs`.
- Keep frontend/client work in the existing pnpm workspace.
- Add new dependencies only when existing crates/packages cannot cover the requirement.
- JSON summaries first; vector retrieval is Phase 2.

## Team Allocation

### Engineer 1: Event Ingestion + Normalization

Ownership:

- Build a single normalized event model used by downstream summarization.
- Implement adapters for:
  - `exec --experimental-json` stream.
  - app-server `turn/*` + `item/*` stream.
  - persisted rollout replay for backfill/debug.

Primary repo areas:

- `codex-rs/exec/`
- `codex-rs/app-server/`
- `codex-rs/app-server-protocol/`
- `sdk/typescript/src/events.ts` and `sdk/typescript/src/items.ts` (for TS parity)

Deliverables:

- `NormalizedEvent` schema and versioning strategy.
- Deterministic mapper from source events -> normalized events.
- Replay harness with fixtures from real sessions.

Definition of done:

- Same turn produces equivalent normalized records across exec-stream and app-server-stream paths.
- Fixture-based tests cover command/file/tool/plan/error lifecycle events.

### Engineer 2: Summary Engine + JSON Schema

Ownership:

- Build aggregation logic from normalized events -> compact session summary JSON.
- Define grouping/promotion rules so graph is readable by default.

Primary repo areas:

- New summary module/crate under `codex-rs` (recommended)
- `codex-rs/app-server-protocol/` if summary types are exposed over app-server
- `docs/` for summary schema and examples

Deliverables:

- `SessionSummary` schema:
  - session metadata
  - turn summaries
  - grouped nodes (`plan`, `execution`, `code_changes`, `external_tools`)
  - evidence pointers (file paths, command+exit, error snippets, event ids)
- Promotion rules for first-class child nodes (failures, major plan pivots, large edits).
- Deterministic summarizer implementation with snapshot fixtures.

Definition of done:

- Summary JSON is stable for the same input event stream.
- Each summary node can map back to source evidence ids/paths.

### Engineer 3: Summary Storage + Query/Retrieval API

Ownership:

- Persist summary artifacts.
- Provide query interfaces for deterministic retrieval now and vector integration later.

Primary repo areas:

- `codex-rs/state/` for metadata/indexing where appropriate
- `codex-rs/app-server/` for optional read/search endpoints
- New storage/query module under `codex-rs`

Deliverables:

- Summary persistence layout (JSON files + lightweight index metadata).
- Query API for:
  - by thread/session id
  - by file path
  - by command substring
  - by error text
- Optional app-server experimental endpoints for external UI consumption.

Definition of done:

- Queries return matching nodes with enough context to jump to raw evidence.
- Storage can handle resumed/forked sessions without duplicating node identity.

### Engineer 4: Realtime Graph UI + Interaction

Ownership:

- Build the separate UI that renders the parent->child graph and updates live.
- Wire search and evidence drill-down.

Primary repo areas:

- New pnpm workspace package for UI (recommended under `sdk/` or top-level package)
- `codex-rs/app-server-test-client/` for local streaming test harness

Deliverables:

- Graph view:
  - root session
  - per-turn parent nodes
  - grouped child nodes with expand/collapse
- Evidence panel showing file changes, commands, errors, and plan updates.
- Realtime updates from stream bridge (app-server or JSONL tail bridge).
- Search UX (file path / command / error substring) that jumps to matching nodes.

Definition of done:

- A running Codex session updates the graph in near-real-time.
- User can quickly locate failure moments and major decision points.

## Shared Contracts (Critical Handoffs)

1. Ingestion -> Summary contract:
   - Freeze `NormalizedEvent` schema first.
   - Include stable ids: `thread_id`, `turn_id`, `item_id`, timestamps.
2. Summary -> UI contract:
   - Freeze `SessionSummary` node shape with explicit `node_type` and `parent_id`.
   - Include evidence references, not full raw transcripts.
3. Storage -> UI/API contract:
   - Return summary nodes plus lookup metadata for drill-down.
   - Keep response shape stable with schema versioning.

## Milestones

1. Milestone 0 (Design freeze):
   - Finalize `NormalizedEvent` and `SessionSummary` schemas.
   - Decide source-of-truth priority (app-server stream vs rollout replay).
2. Milestone 1 (Pipeline functional):
   - Ingest events and emit summary JSON for completed sessions.
3. Milestone 2 (Realtime graph):
   - Live stream updates into graph UI with expandable nodes.
4. Milestone 3 (Retrieval + polish):
   - Deterministic search, quality pass, docs, and demo flow.

## Toolchain and Validation Checklist

- Rust formatting/lint:
  - `just fmt`
  - `just fix -p <crate>`
- Rust tests:
  - `cargo test -p <affected-crate>`
  - If app-server protocol changes: `just write-app-server-schema` and `cargo test -p codex-app-server-protocol`
- TypeScript package checks:
  - `pnpm --filter <package> test`
  - `pnpm --filter <package> build`
- Docs updates:
  - Update `docs/` when API shapes or workflows change.

## Open Decisions (Resolve Early)

- Whether AgentCanvas is read-only over existing rollout files vs adding explicit summary emit hooks in core/app-server.
- Default node budget per turn (how many promoted sub-nodes before collapsing).
- Where to store summaries by default (`CODEX_HOME`-relative vs workspace-local override).
- Vector phase boundaries: embedding model choice, indexing location, and retention policy.
