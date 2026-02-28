import type { Node, Edge } from '@xyflow/react'
import type { AppEvent, CommandExecutionEvent, GraphNodeData, NodeKind } from './types'

type GraphNode = Node<GraphNodeData>

export interface GraphResult {
  nodes: GraphNode[]
  edges: Edge[]
  turnOrder: string[]  // ordered list of turnIds for sequential flow edges
}

export function buildGraph(events: AppEvent[]): GraphResult {
  const nodes: GraphNode[] = []
  const edges: Edge[] = []
  const turnOrder: string[] = []

  let threadId: string | null = null

  // ── First pass: collect successful commands per turn for aggregation ──
  const cmdsByTurn = new Map<string, CommandExecutionEvent[]>()
  const failedCmds = new Set<string>()  // event IDs of failed commands

  for (const event of events) {
    if (event.type === 'CommandExecution') {
      const failed = event.exitCode !== 0 && event.exitCode !== null
      if (failed) {
        failedCmds.add(event.id)
      } else {
        if (!cmdsByTurn.has(event.turnId)) cmdsByTurn.set(event.turnId, [])
        cmdsByTurn.get(event.turnId)!.push(event)
      }
    }
  }

  // ── Second pass: build the graph ──
  const emittedCmdSummary = new Set<string>()  // turnIds that already have a summary node

  for (const event of events) {
    if (event.type === 'ThreadStarted') {
      threadId = event.threadId
      nodes.push({
        id: `session-${event.threadId}`,
        type: 'sessionNode',
        position: { x: 0, y: 0 },
        data: {
          kind: 'session',
          label: event.threadId,
          status: 'running',
          turnId: '',
          rawEvent: event,
        },
      })
    } else if (event.type === 'TurnStarted') {
      if (!threadId) threadId = event.sessionId
      const turnNodeId = `turn-${event.turnId}`
      nodes.push({
        id: turnNodeId,
        type: 'turnNode',
        position: { x: 0, y: 0 },
        data: {
          kind: 'turn',
          label: event.userPrompt,
          status: 'running',
          turnId: event.turnId,
          rawEvent: event,
          collapsed: false,
        },
      })
      turnOrder.push(event.turnId)

      const prevTurnId = turnOrder.length >= 2 ? turnOrder[turnOrder.length - 2] : null
      if (prevTurnId) {
        edges.push({
          id: `flow-${prevTurnId}-${event.turnId}`,
          source: `turn-${prevTurnId}`,
          target: turnNodeId,
          data: { kind: 'flow' },
        })
      } else if (threadId) {
        edges.push({
          id: `flow-session-${event.turnId}`,
          source: `session-${threadId}`,
          target: turnNodeId,
          data: { kind: 'flow' },
        })
      }
    } else if (event.type === 'UserPromptPatch') {
      const node = nodes.find(n => n.id === `turn-${event.turnId}`)
      if (node) node.data = { ...node.data, label: event.userPrompt }
    } else if (event.type === 'TurnComplete') {
      const node = nodes.find(n => n.id === `turn-${event.turnId}`)
      if (node) node.data = { ...node.data, status: event.status }
    } else if (event.type === 'CommandExecution') {
      // Failed commands → individual error nodes
      if (failedCmds.has(event.id)) {
        nodes.push({
          id: `event-${event.id}`,
          type: 'eventNode',
          position: { x: 0, y: 0 },
          data: {
            kind: 'error',
            label: event.cmd.slice(0, 60),
            status: 'error',
            turnId: event.turnId,
            rawEvent: event,
          },
        })
        edges.push({
          id: `detail-${event.id}`,
          source: `turn-${event.turnId}`,
          target: `event-${event.id}`,
          data: { kind: 'detail' },
        })
        continue
      }

      // Successful commands → one aggregate summary node per turn
      if (!emittedCmdSummary.has(event.turnId)) {
        emittedCmdSummary.add(event.turnId)
        const cmds = cmdsByTurn.get(event.turnId) ?? []
        const summaryId = `cmds-${event.turnId}`
        nodes.push({
          id: summaryId,
          type: 'eventNode',
          position: { x: 0, y: 0 },
          data: {
            kind: 'command',
            label: `${cmds.length} command${cmds.length !== 1 ? 's' : ''} run`,
            status: 'success',
            turnId: event.turnId,
            rawEvent: event,
            // Store all commands for the evidence panel
            aggregatedCommands: cmds,
          } as GraphNodeData,
        })
        edges.push({
          id: `detail-${summaryId}`,
          source: `turn-${event.turnId}`,
          target: summaryId,
          data: { kind: 'detail' },
        })
      }
      // Skip individual successful command nodes (already aggregated)
    } else if (event.type === 'McpToolCall') {
      nodes.push({
        id: `event-${event.id}`,
        type: 'eventNode',
        position: { x: 0, y: 0 },
        data: {
          kind: 'tool',
          label: event.toolName,
          status: event.status,
          turnId: event.turnId,
          rawEvent: event,
        },
      })
      edges.push({
        id: `detail-${event.id}`,
        source: `turn-${event.turnId}`,
        target: `event-${event.id}`,
        data: { kind: 'detail' },
      })
    } else if (event.type === 'PatchApply') {
      nodes.push({
        id: `event-${event.id}`,
        type: 'eventNode',
        position: { x: 0, y: 0 },
        data: {
          kind: 'patch',
          label: event.filename,
          status: event.status,
          turnId: event.turnId,
          rawEvent: event,
        },
      })
      edges.push({
        id: `detail-${event.id}`,
        source: `turn-${event.turnId}`,
        target: `event-${event.id}`,
        data: { kind: 'detail' },
      })
    } else if (event.type === 'PlanUpdate') {
      nodes.push({
        id: `event-${event.id}`,
        type: 'eventNode',
        position: { x: 0, y: 0 },
        data: {
          kind: 'plan',
          label: event.text.slice(0, 60),
          status: 'info',
          turnId: event.turnId,
          rawEvent: event,
        },
      })
      edges.push({
        id: `detail-${event.id}`,
        source: `turn-${event.turnId}`,
        target: `event-${event.id}`,
        data: { kind: 'detail' },
      })
    }
  }

  return { nodes, edges, turnOrder }
}
