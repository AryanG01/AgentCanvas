# AgentCanvas Graph UI — Design Document

**Engineer:** 4 (Realtime Graph UI + Interaction)
**Date:** 2026-02-28
**Status:** Approved — implementation in progress on branch `aryan/4`

---

## 1. Scope

Engineer 4 owns the standalone web UI that:

- Connects to a running `codex app-server` over WebSocket.
- Renders session events as a live, interactive parent→child graph.
- Lets the user drill into any node to see raw evidence (commands, diffs, tool I/O).
- Provides text search that highlights matching nodes.

This UI is **read-only and observation-only**. It does not start sessions, approve commands, or modify the agent. Engineers 1–3 own the event ingestion, summary engine, and storage layers in Rust.

---

## 2. Input Format — What We Ingest

### 2.1 Transport

The app-server exposes an **experimental WebSocket transport** at a configurable host/port (default `localhost:8080`). Each WebSocket text frame carries exactly one JSON-RPC 2.0 message (the `"jsonrpc":"2.0"` header is omitted on the wire).

```
ws://localhost:8080
```

All messages are JSON objects. Server-to-client messages are either:

- **Responses** (`id` present) — replies to our requests (we only send `initialize` + `initialized`).
- **Notifications** (`id` absent, `method` present) — the event stream we care about.

### 2.2 Connection Handshake

Before any events flow we must complete the initialize handshake:

```json
// → send (request)
{ "method": "initialize", "id": 0,
  "params": { "clientInfo": { "name": "agentcanvas_ui", "version": "0.1.0" } } }

// ← receive (response)
{ "id": 0, "result": { ... } }

// → send (notification)
{ "method": "initialized", "params": {} }
```

After `initialized` the server starts streaming turn/item notifications.

### 2.3 Notification Events We Consume

All notifications have the shape:

```json
{ "method": "<method>", "params": { ... } }
```

#### `turn/started`

Emitted when a new user turn begins.

```json
{
  "method": "turn/started",
  "params": {
    "threadId": "thr_abc",
    "turn": {
      "id": "turn_xyz",
      "status": "inProgress",
      "items": []
    }
  }
}
```

Key fields: `params.threadId`, `params.turn.id` (the stable `turnId`), `params.turn.status`.

**Note:** The initial `turn` object does NOT include user prompt text. The user message arrives as an `item/completed` notification with a `userMessage` ThreadItem shortly after.

#### `turn/completed`

Emitted when the turn finishes (success, interrupted, or failed).

```json
{
  "method": "turn/completed",
  "params": {
    "threadId": "thr_abc",
    "turn": {
      "id": "turn_xyz",
      "status": "completed"   // | "interrupted" | "failed"
    }
  }
}
```

#### `item/started` and `item/completed`

The primary event stream. Each item goes through `item/started` → (optional deltas) → `item/completed`.

**Always prefer `item/completed` as authoritative** — exit codes, tool results, and patch status are only fully populated there.

```json
{
  "method": "item/completed",
  "params": {
    "threadId": "thr_abc",
    "turnId": "turn_xyz",
    "item": { /* ThreadItem — see §2.4 */ }
  }
}
```

#### `turn/plan/updated`

Emitted whenever the agent updates its plan steps.

```json
{
  "method": "turn/plan/updated",
  "params": {
    "threadId": "thr_abc",
    "turnId": "turn_xyz",
    "explanation": "I will list all TS files then add a log",
    "plan": [
      { "step": "List TypeScript files", "status": "completed" },
      { "step": "Add console.log to main.ts", "status": "inProgress" }
    ]
  }
}
```

#### `turn/diff/updated`

Emitted after every FileChange item; carries the full aggregated unified diff for the turn.

```json
{
  "method": "turn/diff/updated",
  "params": {
    "threadId": "thr_abc",
    "turnId": "turn_xyz",
    "diff": "--- a/src/main.ts\n+++ b/src/main.ts\n..."
  }
}
```

### 2.4 ThreadItem Tagged Union

The `item` field in `item/started` / `item/completed` is a tagged union with a `type` discriminator.

#### `userMessage`

```json
{
  "type": "userMessage",
  "id": "item_001",
  "content": [{ "type": "text", "text": "List all TypeScript files in src/" }]
}
```

We extract the first `text` element as the turn's label/prompt.

#### `commandExecution`

```json
{
  "type": "commandExecution",
  "id": "item_002",
  "command": "find src/ -name '*.ts'",
  "cwd": "/workspace",
  "status": "completed",   // | "inProgress" | "failed" | "declined"
  "output": {
    "type": "commandOutput",
    "exitCode": 0,
    "stdout": "src/main.ts\nsrc/App.tsx",
    "stderr": ""
  }
}
```

Non-zero `exitCode` (or `status: "failed"`) → render as **error** node.

#### `fileChange`

```json
{
  "type": "fileChange",
  "id": "item_003",
  "changes": [
    { "path": "src/main.ts", "type": "edit" }
  ],
  "status": "completed"   // | "inProgress" | "failed" | "declined"
}
```

#### `mcpToolCall`

```json
{
  "type": "mcpToolCall",
  "id": "item_004",
  "server": "filesystem",
  "tool": "read_file",
  "status": "completed",   // | "inProgress" | "failed"
  "arguments": { "path": "src/main.ts" },
  "result": { "content": [{ "type": "text", "text": "..." }] },
  "error": null,
  "durationMs": 42
}
```

#### `plan`

```json
{
  "type": "plan",
  "id": "item_005",
  "text": "I will first list files, then add a log statement."
}
```

#### Other items (ignored for graph, preserved for Evidence Panel)

`agentMessage`, `reasoning`, `webSearch`, `dynamicToolCall`, `contextCompaction`, `enteredReviewMode`, `exitedReviewMode`.

---

## 3. Internal Normalized Event Model

Because the plan's types were designed before reading the actual protocol, we adapt the implementation's internal `AppEvent` union to match the real wire format:

| Internal `type`     | Source notification + ThreadItem type                        |
|---------------------|--------------------------------------------------------------|
| `TurnStarted`       | `turn/started` — `params.turn.id`, label comes from subsequent `userMessage` item |
| `TurnComplete`      | `turn/completed` — `params.turn.status`                     |
| `CommandExecution`  | `item/completed` where `item.type === "commandExecution"`   |
| `McpToolCall`       | `item/completed` where `item.type === "mcpToolCall"`        |
| `PatchApply`        | `item/completed` where `item.type === "fileChange"`         |
| `PlanUpdate`        | `item/completed` where `item.type === "plan"` **or** `turn/plan/updated` |

> **Design decision:** We buffer `TurnStarted` and update its label when the `userMessage` item arrives. This avoids showing an empty/ID-only label on the parent node.

---

## 4. Output Format — What We Render

### 4.1 Graph Node Types

The `buildGraph(events)` pure function maps the normalized event list to two ReactFlow node types:

#### `TurnNode` (type: `"turnNode"`)

| Field        | Value                                      |
|--------------|--------------------------------------------|
| `id`         | `"turn-{turnId}"`                          |
| `data.kind`  | `"turn"`                                   |
| `data.label` | User prompt text (first 80 chars)          |
| `data.status`| `"running"` → `"success"/"error"/"cancelled"` |
| Width        | 220 px                                     |

#### `EventNode` (type: `"eventNode"`)

| Field        | Value                                                     |
|--------------|-----------------------------------------------------------|
| `id`         | `"event-{itemId}"`                                        |
| `data.kind`  | `"command"` / `"tool"` / `"patch"` / `"plan"` / `"error"` |
| `data.label` | Command string / tool name / file path / plan text (60 chars) |
| `data.status`| `"success"` / `"error"` / `"info"`                        |
| Width        | 200 px                                                    |

Kind promotion rule: `commandExecution` with non-zero exit code → `kind: "error"`.

### 4.2 Edges

Each event node gets one directed edge from its parent turn node:

```
"turn-{turnId}" → "event-{itemId}"
```

Edge ID: `"e-{itemId}"`

### 4.3 Layout

Dagre runs `rankdir: "TB"` (top-to-bottom). Nodes are placed with:

- `nodesep: 40` (horizontal gap between siblings)
- `ranksep: 60` (vertical gap between ranks)

Positions are centred: `x = dagre.x - width/2`, `y = dagre.y - height/2`.

### 4.4 Visual Encoding

| Kind      | Dot colour   | Badge      |
|-----------|-------------|------------|
| `turn`    | Yellow (running) / Green (success) / Red (error) | — |
| `command` | Yellow      | `CMD`      |
| `tool`    | Blue        | `MCP`      |
| `patch`   | Green       | `PATCH`    |
| `plan`    | Grey        | `PLAN`     |
| `error`   | Red         | `ERR`      |

### 4.5 Evidence Panel (Drawer)

Clicking any node opens a right-side drawer (380 px wide) showing:

| Node kind | Sections shown                                  |
|-----------|-------------------------------------------------|
| `command` / `error` | Command string, exit code, stdout, stderr |
| `tool`    | Tool name, arguments (JSON), result (JSON)      |
| `patch`   | File path(s), unified diff                      |
| `plan`    | Plan text / step list                           |
| `turn`    | Raw JSON of the full `turn` object              |

### 4.6 Search Behaviour

- Input in `SearchBar` writes to `graphStore.searchQuery`.
- `GraphCanvas` maps nodes: if `searchQuery` is non-empty and `node.data.label` does not contain the query string (case-insensitive), `node.style.opacity = 0.15`.
- Clearing search (× button or backspace) restores full opacity.

---

## 5. State Management

Single Zustand store (`graphStore`):

```
events: AppEvent[]          // append-only normalized event log
nodes: Node<GraphNodeData>[]  // derived, laid out
edges: Edge[]               // derived
selectedNodeId: string | null
searchQuery: string
wsStatus: 'connecting' | 'connected' | 'disconnected'
```

`addEvent(e)` → recomputes `buildGraph` + `applyDagreLayout` on every event. Acceptable for session sizes (hundreds of events); can be optimised to incremental layout later.

---

## 6. WebSocket Hook Behaviour

- Connects to `VITE_WS_URL` (default: `ws://localhost:5173/ws`, proxied to `ws://localhost:8080`).
- On open: sends `initialize` + `initialized` handshake, sets status `"connected"`.
- On message: parses JSON, extracts `params` (or root), checks `type` field, calls `addEvent`.
- On close: exponential backoff reconnect (1 s → 2 s → … → 30 s max).
- On error: closes socket (triggers reconnect).

---

## 7. Tech Stack

| Concern       | Library                     | Why                                      |
|---------------|-----------------------------|------------------------------------------|
| Bundler       | Vite 5                      | Fast HMR, native ESM, in pnpm workspace  |
| UI            | React 18 + TypeScript       | Component model, strict types            |
| Graph         | `@xyflow/react` (ReactFlow v12) | Best-in-class interactive graph, custom nodes |
| Layout        | `dagre`                     | Battle-tested directed-graph layout      |
| State         | Zustand                     | Minimal, no boilerplate, hook-friendly   |
| Styles        | Tailwind CSS v3             | Utility-first, dark theme trivial        |
| Tests         | Vitest                      | Same config as Vite, fast               |

---

## 8. File Structure

```
agentcanvas-ui/
├── src/
│   ├── lib/
│   │   ├── types.ts          # AppEvent union + GraphNodeData
│   │   ├── eventToGraph.ts   # pure: AppEvent[] → { nodes, edges }
│   │   ├── eventToGraph.test.ts
│   │   ├── layout.ts         # pure: nodes + edges → laid-out nodes
│   │   ├── layout.test.ts
│   │   └── mockEvents.ts     # dev/testing fixture
│   ├── store/
│   │   └── graphStore.ts     # Zustand store
│   ├── hooks/
│   │   └── useAppServerWS.ts # WS lifecycle + reconnect
│   ├── components/
│   │   ├── TurnNode.tsx
│   │   ├── EventNode.tsx
│   │   ├── EvidencePanel.tsx
│   │   ├── SearchBar.tsx
│   │   └── GraphCanvas.tsx
│   ├── App.tsx
│   └── index.css             # Tailwind directives + ReactFlow base styles
├── .env.local                # VITE_USE_MOCK=true (dev only, not committed)
├── vite.config.ts            # WS proxy: /ws → ws://localhost:8080
├── tailwind.config.js
└── package.json
```

---

## 9. Open Decisions / Known Gaps

| # | Question | Current answer |
|---|----------|---------------|
| 1 | User prompt text | Buffered from `userMessage` item; shown as `"(loading…)"` until it arrives |
| 2 | Real `turn/plan/updated` format | Plan steps are an array `{ step, status }`, not a single text blob — `PlanUpdate` type needs revision in Task 2 |
| 3 | Multi-session UI | Not in scope for this iteration; single active thread only |
| 4 | Collapse/expand children | Deferred to next iteration; TurnNode data has a `collapsed` flag stub |
| 5 | Engineer 2's `SessionSummary` format | UI will accept either raw app-server stream OR pre-computed summary nodes once E2 delivers the schema |

---

## 10. Verification Checklist

- [ ] `pnpm tsc --noEmit` — zero errors
- [ ] `pnpm vitest run` — 5 tests pass (4 mapper + 1 layout)
- [ ] `pnpm dev` — dev server at `localhost:5173`
- [ ] Mock mode (`VITE_USE_MOCK=true`): 2 turns + children animate in at 300 ms intervals
- [ ] Click any node → EvidencePanel opens with correct raw event JSON
- [ ] Search dims non-matching nodes; × clears
- [ ] WS status dot visible in SearchBar (red in mock mode — no real WS)
