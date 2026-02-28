# AgentCanvas Graph UI — Current State (Feb 28, 2026)

> Plain-English description of what's built, how it works, and what's next.

---

## What It Is

A web app (`agentcanvas-ui/`) that watches a live Codex CLI session and draws everything that happens as an interactive graph — in real time. You can see which commands ran, what files were changed, what the AI was planning, and click any node to see the raw details.

---

## How the Data Flows

```
Codex CLI session
      │
      │  JSON-RPC 2.0 over WebSocket
      ▼
  app-server (ws://localhost:8080)
      │
      │  WebSocket connection (useAppServerWS hook)
      ▼
  Zustand store  ←── events accumulate here
      │
      │  buildGraph() pure function
      ▼
  ReactFlow graph  ←── rendered in browser
```

1. **Codex CLI** runs commands, reads files, writes patches, and streams events to `app-server`.
2. **`app-server`** broadcasts those events as JSON-RPC 2.0 notifications over WebSocket.
3. **Our hook** (`useAppServerWS.ts`) listens on the WebSocket and translates real wire-format messages into our internal `AppEvent` types.
4. Every new event is pushed into the **Zustand store**, which rebuilds the graph (nodes + edges) and re-layouts the canvas automatically.
5. **ReactFlow** renders the graph; clicking a node opens the Evidence Panel with full details.

---

## What the Graph Looks Like

```
  [Session: thr_abc123]          ← purple root node, one per session
         │
         │ (flow edge — thick animated indigo)
         ▼
  [Turn 1: "List TypeScript files"]   ← blue turn node
    ├──── [CMD: find src/ -name *.ts]   ← command child (yellow)
    └──── [PLAN: Found 3 files...]      ← plan child (grey)
         │
         │ (flow edge — turn chain, shows conversation dependency)
         ▼
  [Turn 2: "Add a console.log"]        ← turn node (shown as error, red dot)
    ├──── [MCP: read_file]              ← tool call child (blue)
    ├──── [PATCH: src/main.ts]          ← file edit child (green)
    └──── [ERR: node src/main.ts → exit 1]  ← error child (red)
         │
         ▼
  [Turn 3: "Fix the import error"]     ← turn node (success, green dot)
    ├──── [PLAN: Will use tsx instead]
    └──── [CMD: npx tsx src/main.ts → exit 0]
```

**Two kinds of edges:**
- **Flow edges** (thick, animated indigo): session → turn, turn → next turn. Shows the conversation chain — each turn depends on the one before it.
- **Detail edges** (thin, dashed grey): turn → its tool calls / commands / patches. Shows what happened inside that turn.

---

## Node Types

| Node | Color | Meaning |
|------|-------|---------|
| Session | Indigo gradient | The root of the entire conversation thread |
| Turn | Blue/slate | One user message + AI response pair. Status dot shows: yellow=running, green=success, red=error |
| CMD | Yellow | A shell command the AI ran (`exitCode=0` → ok, `exitCode≠0` → red ERR node) |
| MCP | Blue | A tool call to an MCP server (e.g. `read_file`, `write_file`) |
| PATCH | Green | A file edit the AI applied |
| PLAN | Grey | The AI's planning/reasoning text |
| ERR | Red | A failed command (same as CMD but non-zero exit) |

---

## UI Features

- **Search bar** (top center): type to dim non-matching nodes to 12% opacity
- **WS status dot** (left of search bar): green=connected, yellow=connecting, red=disconnected
- **Node counter** (right of search bar): shows "2 turns · 7 events" or "3 matches" while searching
- **Click a node** → Evidence Panel slides in from the right with raw details:
  - Commands: full stdout/stderr with syntax coloring
  - File patches: diff view (green `+`, red `-`, blue `@@`)
  - MCP tool calls: input + output JSON
- **MiniMap** (bottom right): bird's-eye view with color-coded nodes
- **Zoom/pan controls** (bottom left)
- **ConnectPanel** (bottom center, shown when not connected): paste in a WebSocket URL to connect to a real Codex session. Command snippet included: `codex app-server --listen ws://127.0.0.1:8080`

---

## Mock Mode vs Real Mode

**Mock mode** (`VITE_USE_MOCK=true` in `.env.local`):
- Streams 17 pre-scripted events with 300ms delays
- No WebSocket needed
- Great for development/demo

**Real mode** (default, `VITE_USE_MOCK` not set):
- Shows ConnectPanel with URL input
- Connect to: `ws://localhost:5173/ws` (proxied via Vite to `ws://localhost:8080`)
- Or connect directly to `ws://localhost:8080`
- The hook handles JSON-RPC protocol: listens for `thread/started`, `turn/started`, `turn/completed`, `item/completed` notifications
- Turn labels start as `"(typing…)"` and update retroactively when the user message arrives

---

## File Map

```
agentcanvas-ui/
├── src/
│   ├── lib/
│   │   ├── types.ts          — all TypeScript types (AppEvent union, GraphNodeData, etc.)
│   │   ├── eventToGraph.ts   — pure fn: AppEvent[] → {nodes, edges, turnOrder}
│   │   ├── layout.ts         — applyConversationLayout (turn spine + item branches)
│   │   ├── mockEvents.ts     — 17 scripted events for dev/demo
│   │   ├── eventToGraph.test.ts  — 5 unit tests for graph mapper
│   │   └── layout.test.ts        — 1 unit test for layout
│   ├── store/
│   │   └── graphStore.ts     — Zustand store (events, nodes, edges, WS status, selected node)
│   ├── hooks/
│   │   └── useAppServerWS.ts — WebSocket hook, JSON-RPC parser, exponential backoff reconnect
│   ├── components/
│   │   ├── SessionNode.tsx   — indigo root node
│   │   ├── TurnNode.tsx      — turn node with status dot + label
│   │   ├── EventNode.tsx     — item nodes (CMD/MCP/PATCH/PLAN/ERR) with icons
│   │   ├── GraphCanvas.tsx   — ReactFlow canvas with edge styles
│   │   ├── EvidencePanel.tsx — right-side drawer with raw event detail
│   │   ├── SearchBar.tsx     — search + WS status + node counter
│   │   └── ConnectPanel.tsx  — WS URL input for real mode
│   ├── App.tsx               — root component, mock event streaming
│   ├── main.tsx              — entry point (imports ReactFlow CSS here)
│   └── index.css             — Tailwind base/components/utilities
├── vite.config.ts            — Vite config + Vitest config + WS proxy
├── tailwind.config.js        — Tailwind v3 content paths
└── .env.local                — VITE_USE_MOCK=true (for local dev)
```

---

## What's Connected to the Rest of the Codebase

| Engineer | Component | Integration Point |
|----------|-----------|-------------------|
| E1 | `codex-rs/agentcanvas/` — NormalizedEvent schema | We consume these events indirectly via app-server |
| E3 | `codex-rs/state/` — SessionSummary storage | Data saved by E3 persists; E4 reads live stream, not stored data |
| E2 | Summary Engine | **Not yet implemented.** When E2 delivers `SessionSummary` events, we can swap LLM summaries into turn node labels instead of raw user prompts |

---

## What's Left To Do

1. **E2 Summary Engine**: When E2 ships a `SessionSummary` event type, add it to `types.ts` and display the summary in the TurnNode label instead of the truncated user prompt.
2. **Collapse/expand turns**: Add a toggle on TurnNode to hide/show child events to reduce clutter in long sessions.
3. **Session picker**: List past `~/.codex/sessions/` files and let the user replay them in the graph.
4. **Live test against real app-server**: Start `codex app-server --listen ws://127.0.0.1:8080`, open `localhost:5173`, connect via the ConnectPanel, and verify the graph updates as commands run.
