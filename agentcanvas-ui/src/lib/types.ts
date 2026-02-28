// ---------------------------------------------------------------------------
// Internal normalized AppEvent union
// These are our internal types — the WS hook maps real protocol JSON into these.
// ---------------------------------------------------------------------------

export type WsStatus = 'connecting' | 'connected' | 'disconnected'

/** Emitted once per codex app-server session/thread. Creates the root node. */
export interface ThreadStartedEvent {
  type: 'ThreadStarted'
  threadId: string
  ts: number
}

/**
 * Emitted when a new user turn begins (from `turn/started` notification).
 * NOTE: userPrompt starts as '(typing…)' — it is patched in when the
 * `userMessage` item arrives via `item/completed`.
 */
export interface TurnStartedEvent {
  type: 'TurnStarted'
  turnId: string
  sessionId: string   // threadId
  userPrompt: string
  ts: number
}

export interface TurnCompleteEvent {
  type: 'TurnComplete'
  turnId: string
  status: 'success' | 'error' | 'cancelled'
  ts: number
}

export interface CommandExecutionEvent {
  type: 'CommandExecution'
  id: string
  turnId: string
  cmd: string
  exitCode: number | null
  stdout: string
  stderr: string
  ts: number
}

export interface McpToolCallEvent {
  type: 'McpToolCall'
  id: string
  turnId: string
  toolName: string
  server?: string
  input: unknown
  output: unknown
  status: 'success' | 'error'
  ts: number
}

export interface PatchApplyEvent {
  type: 'PatchApply'
  id: string
  turnId: string
  filename: string
  diff: string
  status: 'success' | 'error'
  ts: number
}

export interface PlanUpdateEvent {
  type: 'PlanUpdate'
  id: string
  turnId: string
  text: string
  ts: number
}

export interface SummaryNodeEvent {
  type: 'SummaryNode'
  id: string
  turnId: string
  status: string
  summaryText: string
  brief: {
    signal: string
    agentMessage: string | null
    primaryCommand: string | null
    primaryFilePath: string | null
    primaryError: string | null
  }
  counts: {
    commandsTotal: number
    commandsIndexed: number
    commandsOmitted: number
    filePathsTotal: number
    filePathsIndexed: number
    filePathsOmitted: number
    errorsTotal: number
    errorsIndexed: number
    errorsOmitted: number
  }
  digest: {
    commandExamples: string[]
    filePathExamples: string[]
    errorExamples: string[]
  }
  lineage: {
    parentTurnId: string | null
    forkedFromThreadId: string | null
    startedAfterRollback: boolean
    wasRolledBack: boolean
  }
  evidence: {
    filePaths: string[]
    commands: Array<{ command: string; exitCode: number | null }>
    errors: string[]
  }
  ts: number
}

/** Internal synthetic event — updates a turn node's label from the userMessage item */
export interface UserPromptPatchEvent {
  type: 'UserPromptPatch'
  turnId: string
  userPrompt: string
  ts: number
}

export type AppEvent =
  | ThreadStartedEvent
  | TurnStartedEvent
  | TurnCompleteEvent
  | CommandExecutionEvent
  | McpToolCallEvent
  | PatchApplyEvent
  | PlanUpdateEvent
  | SummaryNodeEvent
  | UserPromptPatchEvent

// ---------------------------------------------------------------------------
// Graph node types
// ---------------------------------------------------------------------------

export type NodeKind = 'session' | 'turn' | 'command' | 'tool' | 'patch' | 'plan' | 'error' | 'summary'

export interface GraphNodeData extends Record<string, unknown> {
  kind: NodeKind
  label: string
  status?: string
  turnId: string
  rawEvent: AppEvent
  collapsed?: boolean
  itemCount?: number  // number of child items (for turn nodes)
  aggregatedCommands?: CommandExecutionEvent[]  // for command summary nodes
}

// ---------------------------------------------------------------------------
// Edge kinds — used to drive visual styling
// ---------------------------------------------------------------------------
export type EdgeKind = 'flow' | 'detail'

// ---------------------------------------------------------------------------
// Session info returned by the replay server's /api/sessions endpoint
// ---------------------------------------------------------------------------
export interface SessionInfo {
  date: string
  time: string
  turns: number
  source: string
  cwd: string
  file: string
  id: string
}
