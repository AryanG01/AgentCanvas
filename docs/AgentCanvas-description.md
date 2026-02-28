# AgentCanvas plan for Codex CLI session summaries and real-time graph UI

## Cleaned-up explanation to share with the team

AgentCanvas is a lightweight “observability + memory” layer for Codex CLI. While developers work normally in Codex CLI, AgentCanvas captures the session’s key events (prompts, plans, tool actions like commands and file edits, and outcomes) and converts them into a compact, structured summary that preserves only the information needed to understand what happened and why. Codex CLI already “surfaces a transcript of its actions” and stores session transcripts locally to support resuming conversations, so AgentCanvas builds on top of that existing behavior rather than reinventing logging from scratch. citeturn2view5turn6view0

Instead of forcing someone to read a long transcript, AgentCanvas renders the summary as an expandable parent→child graph. High-level nodes show the timeline of the session (e.g., each user turn and the resulting plan + execution). Expanding a node reveals the underlying evidence (files changed, key commands, key errors, and plan updates). This makes it fast to locate “the moment things went wrong” or “the step where the important decision happened,” without drowning in every micro-action. citeturn6view0turn8view0

The summary is stored as JSON first (for easy iteration and stable debugging). Later, we can add vector database indexing so both humans and the agent can retrieve the most relevant past steps semantically (e.g., “show me the earlier workaround we tried for this failing test”), which helps long-running sessions and context-window constraints. This aligns with how Codex gathers an ongoing record of work and may compact context over time to keep sessions moving. citeturn8view0turn8view1

## What we are building on in Codex CLI

Codex CLI is entity["company","OpenAI","ai company, san francisco"]’s local coding agent that can read, change, and run code in a selected directory, and it is open source and primarily written in Rust. citeturn7search19turn1view1 The upstream repository lives on entity["company","GitHub","code hosting, san francisco"] and is Apache-2.0 licensed, which is why forking and modifying the CLI directly is feasible for AgentCanvas. citeturn1view1

Two Codex mechanisms matter a lot for AgentCanvas:

Codex stores transcripts locally so sessions can be resumed, and the resume picker can show a run’s summary; session IDs can be found via the resume UI, `/status`, or in files under `~/.codex/sessions/`. citeturn2view5 This means there is already a durable on-disk “source of truth” we can read and summarize.

Codex can emit a machine-readable JSONL stream of events in non-interactive mode (`codex exec --json`). The stream includes thread/turn lifecycle and item lifecycle events, and item types include agent messages, reasoning, command execution, file changes, web searches, MCP tool calls, and plan updates. citeturn6view0 That event taxonomy is exactly what we want to map into graph nodes.

At the “rich client” layer, Codex also provides an app-server protocol designed to power clients like the VS Code extension, including conversation history, approvals, and streamed agent events. It supports JSON-RPC over stdio (JSONL) and an experimental WebSocket mode, and can generate version-matched TypeScript or JSON Schema artifacts. citeturn6view1 This gives us a second viable integration point for real-time graphs.

## Summary storage model and why JSON is the right first step

The core design principle is: **separate raw logs from human-usable summaries**.

Raw transcripts and event logs are valuable for replay and full debugging, but they are too verbose for reading and too large for persistent “memory” use. The JSON summary should therefore be an intentionally small representation that answers:

What was the user trying to do?
What plan did the agent commit to?
What actions did it take (commands, file edits, external tool calls)?
What changed in the workspace?
What was the outcome, and what is left unresolved?

Codex already emits (or stores) many of these elements as structured events (turn started/completed, item started/completed, file change items, plan updates). citeturn6view0turn2view3 AgentCanvas’s value is to **aggregate these into a hierarchy** and preserve just enough detail to navigate the work.

A practical first JSON shape is “session → turns → node groups,” where each group is compact and typed. The summary should include:

Session metadata: session/thread ID, timestamps, workspace path, model, safety/sandbox settings.
This is important because Codex behavior changes based on configuration. Codex uses layered config files (user `~/.codex/config.toml`, project `.codex/config.toml`, plus CLI overrides), and exposes sandbox/approval controls in the CLI reference. citeturn4view1turn1view4

Turn summary: the user instruction, the final outcome, and a short structured “what happened” list (e.g., files modified, commands executed, key errors). Turn lifecycle and usage (token counts) exist in the JSONL event stream (e.g., `turn.completed` with usage). citeturn6view0

Evidence pointers (not full dumps): file paths changed and diff stats; command strings and exit codes; links to the raw transcript lines / event IDs that contain the full details if needed. The non-interactive JSON stream already distinguishes item types like “command_execution” and “file changes,” which makes this tractable. citeturn6view0turn6view0

If you want the “summary JSON” to be model-generated (rather than purely computed), Structured Outputs can enforce schema compliance. Codex CLI itself supports requesting a final response that conforms to a JSON Schema via `--output-schema` in automation contexts, and the OpenAI docs describe Structured Outputs as enforcing adherence to your JSON schema to prevent missing keys or invalid enums. citeturn6view0turn10view0 In practice, the fastest hackathon approach is usually **hybrid**: compute factual fields from events (files/commands/errors), and optionally use a small structured-output pass to generate a short “rationale” string.

## Graph design and what qualifies as a node

Your concern is correct: if “every action” becomes a node, the graph becomes unreadable. The right solution is to treat the graph like a code editor outline: **default to a compact outline, with progressive disclosure when you expand**.

A useful mental model is: the graph’s primary spine mirrors the agent loop (input → plan → tool actions → observations → updated plan → output). This is the same loop Codex describes as the “agent loop” or harness orchestrating user, model, and tools. citeturn1view2

Recommended node granularity:

Session node (root): one per Codex run/thread.

Turn nodes (children of session): one per user “turn” (prompt). Codex’s event stream and protocol both explicitly model turns (`turn.started`, `turn.completed`, etc.), which makes this a natural parent unit. citeturn6view0turn6view1

Within each turn, create only a small number of “group nodes” by default:
Plan node: the initial plan and major plan updates (collapsed by default).
Execution node: a roll-up of commands run + their outcomes.
Code changes node: a roll-up of files modified + brief change summaries (e.g., “edited 3 files, added tests, fixed import path”).
External tools node: web searches / MCP tool calls (if present).

This is justified by Codex’s own JSONL typology: it emits item types for plan updates, command executions, file changes, web searches, and tool calls, so we are grouping along boundaries Codex already recognizes. citeturn6view0turn2view3

Only “promote” a sub-action into its own first-class node when it has debugging value. Examples:
a failing command (non-zero exit) becomes its own node with stderr summarized,
a file edit that changes many lines or touches critical files becomes its own node,
a plan revision becomes its own node if it materially changes direction.

Branching and backtracking should be first-class. Codex already supports forking sessions while preserving the original transcript (`codex fork`), and the CLI has a backtrack workflow (e.g., `Ctrl+T`) that forks a conversation at an earlier user turn by truncating the rollout JSONL and starting a new branch. citeturn2view0turn9view0 This maps cleanly into a graph where edges represent “continued from” or “forked from,” rather than forcing everything into a single linear timeline.

## Real-time UI architecture for the graph

The goal is a separate UI that updates live while Codex runs, but remains optional for the user. Codex itself should still be able to use the saved summaries as “memory” when continuing work.

There are two practical “real-time feed” options, both supported by how Codex is documented today:

File-tail approach: Codex writes local session artifacts under `~/.codex/sessions/`, and Codex explicitly tells users they can obtain session IDs via files under that directory. citeturn2view5 AgentCanvas can watch the active session’s JSONL as it grows and incrementally update the graph. This requires minimal changes to the CLI, but depends on reliable “append as you go” behavior.

Event-stream approach: run the session through a stream designed for machine consumption. In automation mode, `codex exec --json` outputs a JSONL stream with one JSON object per state change, including thread/turn events and item events with typed payloads. citeturn6view0 For richer interactivity, Codex app-server is specifically designed for clients that want streamed agent events and approvals; it supports JSON-RPC over stdio and an experimental WebSocket mode. citeturn6view1

For a hackathon, the cleanest “separate UI + real time” pairing is often: **consume a stream → update a local web UI over WebSocket** (either by connecting directly to app-server’s WebSocket mode or by having a tiny local bridge that reads JSONL and broadcasts updates).

image_group{"layout":"carousel","aspect_ratio":"16:9","query":["real-time execution graph UI for agent traces","node graph debugger interface for tools and file changes","observability trace waterfall with expandable nodes UI"],"num_per_query":1}

The UI goal is not just a pretty graph; it is *navigation + fast retrieval*. The default view should answer: “What were the major steps?” Clicking expands to show evidence. Search should jump to nodes by file path, command string, or error message (partial string match first; semantic search later when embeddings are added). citeturn6view0turn2view3

## Why vector database integration matters and how it relates to context management

The “future vector DB integration” is not just a buzzword; it directly supports the two hardest problems in agentic coding:

Finding relevant past decisions quickly.
Keeping long sessions working when context limits are reached.

Codex explicitly gathers context from file contents, tool output, and an ongoing record of what it has done, but all information must fit within the model’s context window. Codex may compact context by summarizing relevant information and discarding less relevant details so it can continue over many steps. citeturn8view0 In the broader OpenAI API terminology, “context management” is a first-class concept: server-side compaction is configured under `context_management`, and the purpose of compaction is to reduce context size while preserving the state needed for subsequent turns. citeturn8view1

AgentCanvas summaries become a human-interpretable and machine-retrievable memory layer that complements compaction:

Short term: JSON summaries support deterministic retrieval (“show me the last failing test command and the files edited afterward”) without requiring extra model calls.

Long term: embeddings + vector search let you retrieve semantically relevant nodes across sessions (“the earlier approach we tried to fix flaky CI timeouts”) even if the exact keywords differ. This is much harder with plain keyword search.

Agent-in-the-loop: when Codex needs to continue work after a fork/resume or compaction, it can pull the top-k relevant summary nodes and re-inject them as context. This is precisely what “context management” means in this setting: selecting, compressing, and injecting the right prior state to stay within context limits while maintaining task continuity. citeturn8view1turn8view0

A useful supporting analogy is distributed tracing: the key is correlating many small events into a navigable structure. entity["organization","OpenTelemetry","observability framework"] defines ways to correlate logs and traces by including trace and span IDs in log records, enabling navigation between high-level traces and detailed events. citeturn8view2 Similarly, entity["organization","W3C","web standards org"] Trace Context standardizes how trace identity is propagated so events can be tied back to the same request across components. citeturn8view4 AgentCanvas is effectively applying these observability ideas to an agent run: a session is the trace, turns/actions are spans/events, and the graph is the UI for correlation.

## Terminology alignment and open design decisions

Context management is a correct term here, and it matches OpenAI’s formal wording (`context_management` for compaction and related controls). citeturn8view1 In day-to-day team communication, you may also find “memory layer” or “context retrieval layer” clearer, because it emphasizes *selection and retrieval* rather than “storing everything.”

The remaining design choices that matter most for implementation clarity are:

Where the “truth” lives: whether AgentCanvas is purely a reader of existing Codex session JSONL under `~/.codex/sessions/`, or whether your fork will add an explicit “summary emitter” and “active session ID” signal for external tooling. Codex has an active ecosystem of issues around session logs and resuming, and internal workflows (fork/backtrack) already operate by reading and truncating rollout JSONL. citeturn2view5turn9view0

How summaries are produced: deterministic aggregation from events first (fast, reliable, cheap), with optional structured-output summarization for short natural-language rationale fields. The Structured Outputs docs emphasize schema adherence and type-safety, which is useful if you do any model-based summarization. citeturn10view0

Node budget defaults: establishing a default “node cap” per turn via grouping and promotion rules, so the graph is readable without manual filtering. The grouping strategy above is grounded in the event types Codex already emits (plan updates, command executions, file changes, tool calls). citeturn6view0turn2view3

If you adopt these as explicit team decisions, the rest of the implementation becomes straightforward: capture structured events → aggregate into a compact JSON summary → render as a hierarchical graph → (later) embed nodes for semantic retrieval to support long-horizon context management. citeturn8view0turn8view1