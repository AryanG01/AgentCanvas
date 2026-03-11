---
name: agentcanvas-hybrid-memory-search
description: Search past Codex/AgentCanvas chat history using the local SQLite summary index with hybrid semantic and lexical ranking. Use when the user asks to find previous attempts, commands, file edits, errors, or decisions from earlier chats/sessions, including when exact keywords may differ.
---

# AgentCanvas Hybrid Memory Search

## Objective
Retrieve relevant nodes from past Codex chats and summarize what was previously attempted, failed, or succeeded.

## Quick Start
- Run hybrid search:
  - `python3 .codex/skills/agentcanvas-hybrid-memory-search/scripts/search_agentcanvas_memory.py "<query>" --limit 8`
- Return JSON for structured analysis:
  - `python3 .codex/skills/agentcanvas-hybrid-memory-search/scripts/search_agentcanvas_memory.py "<query>" --limit 8 --json`
- Filter to a specific thread:
  - `python3 .codex/skills/agentcanvas-hybrid-memory-search/scripts/search_agentcanvas_memory.py "<query>" --thread-id <thread_id>`
- Force ranking mode for debugging:
  - `--mode semantic` or `--mode lexical`

## Workflow
1. Derive 1-3 high-signal query strings from the user request.
2. Run hybrid search first.
3. If relevance is weak, rerun with semantic-only and lexical-only modes.
4. Select the strongest matches and synthesize the prior work into a short actionable summary.
5. Cite IDs needed for follow-up retrieval: `thread_id`, `session_id`, `summary_id`, `node_id`.

## Query Crafting
- Include concrete tokens from likely evidence:
  - command text (`cargo test -p codex-state`)
  - file paths (`state/src/runtime/summaries.rs`)
  - error text (`database is locked`)
  - task intent (`retry flaky checks`, `rollback summary node`)
- Prefer multi-token queries over single words.

## Response Format
For each selected memory hit, report:
- identity: `thread_id`, `session_id`, `summary_id`, `node_id`
- scores: hybrid/semantic/lexical
- evidence: file path, command, or error text if present
- takeaway: one line on why this hit matters now

## Troubleshooting
- If the DB path is wrong, pass `--db <path>` or `--codex-home <dir>`.
- If summary tables are missing, the local state DB was created before summary migrations.
- If tables exist but are empty, run additional Codex sessions with sqlite enabled so summaries are persisted.
- For schema/storage details, read `references/storage-and-index-notes.md`.
