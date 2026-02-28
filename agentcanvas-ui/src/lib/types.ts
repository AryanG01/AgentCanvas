// ---------------------------------------------------------------------------
// App-server v2 protocol events (subset we care about)
// ---------------------------------------------------------------------------

export type WsStatus = 'connecting' | 'connected' | 'disconnected'

/**
 * Emitted when a new user turn begins.
 * NOTE: userPrompt is NOT in the wire protocol's turn/started notification.
 * It arrives via a subsequent userMessage item and is patched in by the store.
 */
export interface TurnStartedEvent {
  type: 'TurnStarted'
  turnId: string
  sessionId: string
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

export type AppEvent =
  | TurnStartedEvent
  | TurnCompleteEvent
  | CommandExecutionEvent
  | McpToolCallEvent
  | PatchApplyEvent
  | PlanUpdateEvent

// ---------------------------------------------------------------------------
// Graph node types
// ---------------------------------------------------------------------------

export type NodeKind = 'turn' | 'command' | 'tool' | 'patch' | 'plan' | 'error'

export interface GraphNodeData {
  kind: NodeKind
  label: string
  status?: string
  turnId: string
  rawEvent: AppEvent
  collapsed?: boolean // for TurnNode: are children hidden? (future use)
}
