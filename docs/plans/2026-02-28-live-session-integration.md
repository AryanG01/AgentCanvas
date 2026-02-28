# AgentCanvas Live Session Integration Plan

> **Status tracker** — update this file as work progresses.

---

## Architecture Reality (discovered via codebase exploration)

**Critical finding**: The `codex` TUI and `codex app-server` are **completely separate processes with separate sessions**. The TUI creates its own `CodexThread` directly via `ThreadManager`; the app-server creates *different* `CodexThread` instances for remote clients. Running both simultaneously does NOT share a session — they would be two independent Codex sessions.

This means: you cannot point AgentCanvas at `codex app-server` while the user chats in the TUI and have them observe the same session.

---

## Two Viable Approaches

### Option A — Rollout File Watcher ✅ RECOMMENDED
**No recompile needed. TUI stays 100% unmodified.**

How it works:
1. User runs `codex` in TUI mode as normal
2. The TUI writes every event to `~/.codex/sessions/YYYY/MM/DD/rollout-{timestamp}-{thread_id}.jsonl` as it happens (flush after every line)
3. A small Node.js server watches this directory, tails the active file, converts rollout lines → AppEvents, and serves them via WebSocket on port 8080
4. AgentCanvas UI connects to ws://localhost:8080 and shows the live graph

**Tradeoff**: ~100-500ms delay (file-write latency). Acceptable for observability.

### Option B — Patch Rust TUI + Recompile
**Real-time. Requires Rust code changes and `cargo build`.**

How it works:
1. Modify `codex-rs/tui/src/chatwidget/agent.rs` — in the event loop that receives `CodexThread` events, also send each event to a WS broadcaster
2. Add a small Tokio WS server alongside the TUI process
3. `cargo build -p codex-tui --release` (takes ~5-10 min first time)
4. Run the modified TUI; events stream to both the terminal UI and AgentCanvas

**Tradeoff**: Need to write Rust code and maintain a fork of the TUI. Build time is slow.

---

## Decision: Implement Option A First

Option A is faster to build, keeps us out of Rust internals, and gives a working demo. If real-time (sub-100ms) becomes a hard requirement, we revisit Option B.

---

## Rollout File Format (what we need to parse)

Files live at: `~/.codex/sessions/YYYY/MM/DD/rollout-TIMESTAMP-THREAD_ID.jsonl`

Each line is a JSON object. Key types we care about:

| Rollout line shape | Maps to AppEvent |
|---|---|
| `{"id":"...", "timestamp":"...", "instructions":"..."}` | `ThreadStarted` (first line = session meta, has thread id in filename) |
| `{"record_type":"state"}` | Separator — skip |
| `{"type":"message","role":"user","content":[{"type":"input_text","text":"..."}]}` | `TurnStarted` / `UserPromptPatch` |
| `{"type":"message","role":"assistant","content":"..."}` | Skip (agent reply, not needed for graph) |
| `{"type":"function_call","name":"shell","arguments":"..."}` | `CommandExecution` (pending) |
| `{"type":"function_call_output","output":"{\"exitCode\":0,...}"}` | Completes `CommandExecution` |
| `{"type":"function_call","name":"apply_patch","arguments":"..."}` | `PatchApply` |
| MCP tool calls | `McpToolCall` |

> **Note**: The exact field names must be verified by reading real rollout files before implementing the converter. Run: `ls -lt ~/.codex/sessions/**/*.jsonl | head -3` then `head -40 <newest file>`.

---

## Work Items

### [ ] W1 — Understand real rollout format
**Owner**: unassigned
**Effort**: 30 min
Read the 3 most recent `~/.codex/sessions/` JSONL files. Document the exact JSON shapes for:
- Session start
- User turn start
- Command execution (start + end)
- File patch
- MCP tool call

Update the table above with real field names.

**Files to read**:
- `codex-rs/agentcanvas/src/adapters/rollout_replay.rs` — E1's existing parser
- `codex-rs/protocol/src/protocol.rs` — RolloutLine/RolloutItem types
- `~/.codex/sessions/2026/02/28/*.jsonl` — real examples

---

### [ ] W2 — Write rollout watcher server
**Owner**: E4 (agentcanvas-ui)
**Effort**: 2-3 hrs
**File**: `agentcanvas-ui/scripts/rollout-watcher.mjs`

```
Input:  ~/.codex/sessions/ directory
Output: WebSocket server on ws://localhost:8080
```

Logic:
1. On start, list all `.jsonl` files sorted by mtime → build session list
2. Send `session/list` notification to any connected UI client with `[{id, path, preview, ts}]`
3. When UI sends `session/select {id}`, open that file and tail it:
   - Read all existing lines → convert → emit as AppEvents
   - `fs.watch` for new bytes → read new lines → convert → emit
4. Also watch `~/.codex/sessions/` for new files → emit updated `session/list`

Protocol over WS (simple, not JSON-RPC):
```jsonc
// Server → UI
{ "type": "session/list", "sessions": [{id, preview, ts, path}] }
{ "type": "event", "event": { ...AppEvent } }

// UI → Server
{ "type": "session/select", "id": "rollout-2026-02-28T..." }
```

Converter function `rolloutLineToAppEvent(line, threadId)` — maps rollout line types to our AppEvent union.

---

### [ ] W3 — Add session picker to UI
**Owner**: E4 (agentcanvas-ui)
**Effort**: 1-2 hrs
**File**: `agentcanvas-ui/src/components/ConnectPanel.tsx`

Currently ConnectPanel only shows a WS URL input. Add:
- "Browse local sessions" button that connects to ws://localhost:8080 (rollout watcher)
- After connecting, if server sends `session/list`, show a picker list
- Clicking a session sends `session/select` and the graph starts populating
- Keep the manual URL input for connecting to a real app-server if available

**Files to update**:
- `agentcanvas-ui/src/components/ConnectPanel.tsx`
- `agentcanvas-ui/src/store/graphStore.ts` — add `sessionList` + `selectSession()` action
- `agentcanvas-ui/src/hooks/useAppServerWS.ts` — handle `session/list` and `event` message types

---

### [ ] W4 — Update useAppServerWS hook
**Owner**: E4 (agentcanvas-ui)
**Effort**: 1 hr
**File**: `agentcanvas-ui/src/hooks/useAppServerWS.ts`

Currently assumes standard JSON-RPC protocol (app-server v2). Needs to also handle our rollout watcher protocol:
- If message has `type: "session/list"` → call new store action
- If message has `type: "event"` → call `addEvent(event)`
- Keep existing JSON-RPC handling for real app-server connections

---

### [ ] W5 — Wire up npm script + README
**Owner**: E4
**Effort**: 30 min

Add to `agentcanvas-ui/package.json`:
```json
"scripts": {
  "watch": "node scripts/rollout-watcher.mjs",
  "dev:live": "concurrently \"pnpm watch\" \"pnpm dev\""
}
```

Add `concurrently` as dev dep. Write usage instructions in README or this doc.

---

### [ ] W6 (Optional) — Option B: TUI WebSocket patch
**Owner**: whoever owns codex-rs/tui
**Effort**: 4-6 hrs (Rust)
**Files**:
- `codex-rs/tui/src/chatwidget/agent.rs` — add WS broadcast in event loop
- `codex-rs/tui/src/app.rs` — start WS server on startup
- Build: `cargo build -p codex-tui --release` from `codex-rs/`

Only needed if Option A's delay is unacceptable.

---

## Running Instructions (once W1-W5 are done)

**Terminal 1** — start the rollout watcher:
```bash
cd agentcanvas-ui
node scripts/rollout-watcher.mjs
```

**Terminal 2** — start the Vite dev server:
```bash
cd agentcanvas-ui
pnpm dev
```

**Terminal 3** — run Codex as normal:
```bash
codex
# chat with it — events appear in the browser graph automatically
```

**Browser**: open http://localhost:5173 → click "Browse local sessions" → pick your active session.

---

## Current Status

| Item | Status | Notes |
|------|--------|-------|
| W1 rollout format | ⬜ TODO | Must do first |
| W2 watcher server | ⬜ TODO | Depends on W1 |
| W3 session picker UI | ⬜ TODO | |
| W4 WS hook update | ⬜ TODO | |
| W5 scripts + docs | ⬜ TODO | |
| W6 Rust TUI patch | ⬜ OPTIONAL | Only if needed |

---

## Open Questions

1. Do rollout files flush immediately on each event, or batched? (affects latency)
2. Is there a lock file or `.active` marker we can use to detect the live session automatically instead of requiring user selection?
3. What is the exact JSON shape of `function_call` and `function_call_output` in real rollout files?
