import type { Node, Edge } from '@xyflow/react'
import type { AppEvent, GraphNodeData, NodeKind } from './types'

type GraphNode = Node<GraphNodeData>

export function buildGraph(events: AppEvent[]): { nodes: GraphNode[]; edges: Edge[] } {
  const nodes: GraphNode[] = []
  const edges: Edge[] = []

  for (const event of events) {
    if (event.type === 'TurnStarted') {
      nodes.push({
        id: `turn-${event.turnId}`,
        type: 'turnNode',
        position: { x: 0, y: 0 }, // layout overwrites this
        data: {
          kind: 'turn',
          label: event.userPrompt.slice(0, 80),
          status: 'running',
          turnId: event.turnId,
          rawEvent: event,
          collapsed: false,
        },
      })
    } else if (event.type === 'TurnComplete') {
      const node = nodes.find(n => n.id === `turn-${event.turnId}`)
      if (node) node.data = { ...node.data, status: event.status }
    } else if (event.type === 'CommandExecution') {
      const kind: NodeKind = event.exitCode !== 0 && event.exitCode !== null ? 'error' : 'command'
      nodes.push({
        id: `event-${event.id}`,
        type: 'eventNode',
        position: { x: 0, y: 0 },
        data: {
          kind,
          label: event.cmd.slice(0, 60),
          status: event.exitCode === 0 ? 'success' : 'error',
          turnId: event.turnId,
          rawEvent: event,
        },
      })
      edges.push({
        id: `e-${event.id}`,
        source: `turn-${event.turnId}`,
        target: `event-${event.id}`,
      })
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
        id: `e-${event.id}`,
        source: `turn-${event.turnId}`,
        target: `event-${event.id}`,
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
        id: `e-${event.id}`,
        source: `turn-${event.turnId}`,
        target: `event-${event.id}`,
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
        id: `e-${event.id}`,
        source: `turn-${event.turnId}`,
        target: `event-${event.id}`,
      })
    }
  }

  return { nodes, edges }
}
