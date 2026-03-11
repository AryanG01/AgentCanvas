import { Position, type Node, type Edge } from '@xyflow/react'
import type { GraphNodeData, NodeKind } from './types'

// Node dimensions
const SESSION_H = 56
const TURN_W = 240
const TURN_H = 90    // taller to show prompt
const ITEM_W = 240
const ITEM_H = 52
const ITEM_ROW_GAP = 12   // vertical gap between items in same column
const ITEM_COL_GAP = 24   // horizontal gap between item columns
const TURN_GAP = 72       // horizontal gap between turn clusters
const ITEMS_LEFT_MARGIN = 60  // gap from right edge of turn to first item column
const SESSION_MARGIN_RIGHT = 84
const CANVAS_TOP = 120

// How many items per column before wrapping to a new column
const ITEMS_PER_COL = 4

const ITEM_KIND_ORDER: Record<NodeKind, number> = {
  summary: 0,
  phase: 1,
  command: 2,
  tool: 3,
  patch: 4,
  plan: 5,
  error: 6,
  turn: 7,
  session: 8,
}

function sortTurnItems(nodes: Node<GraphNodeData>[]): Node<GraphNodeData>[] {
  return [...nodes].sort((a, b) => {
    const kindDelta = (ITEM_KIND_ORDER[a.data.kind] ?? 99) - (ITEM_KIND_ORDER[b.data.kind] ?? 99)
    if (kindDelta !== 0) return kindDelta
    return a.data.label.localeCompare(b.data.label)
  })
}

/**
 * Custom conversation-flow layout:
 * [Session] -> [Turn 1] -> [Turn 2] -> [Turn 3] ...
 *               |            |
 *            [items]      [items]
 *
 * Turns form a left-to-right spine.
 * Turn details branch to the right of each turn in compact columns.
 */
export function applyConversationLayout(
  nodes: Node<GraphNodeData>[],
  edges: Edge[]
): Node<GraphNodeData>[] {
  void edges
  const sessionNode = nodes.find(n => n.data.kind === 'session')
  const turnNodes = nodes.filter(n => n.data.kind === 'turn')
  const itemNodes = nodes.filter(n => n.data.kind !== 'session' && n.data.kind !== 'turn')

  const result: Node<GraphNodeData>[] = []

  // Session starts the horizontal spine.
  if (sessionNode) {
    result.push({
      ...sessionNode,
      position: { x: 0, y: CANVAS_TOP + (TURN_H - SESSION_H) / 2 },
      sourcePosition: Position.Right,
    })
  }

  let curX = (sessionNode ? SESSION_W + SESSION_MARGIN_RIGHT : 0)

  for (const turn of turnNodes) {
    const children = sortTurnItems(itemNodes.filter(n => n.data.turnId === turn.data.turnId))

    // Calculate detail block dimensions
    const cols = Math.ceil(children.length / ITEMS_PER_COL)
    const rowsInFirstCol = Math.min(children.length, ITEMS_PER_COL)
    const itemBlockH =
      rowsInFirstCol === 0
        ? 0
        : rowsInFirstCol * ITEM_H + Math.max(0, rowsInFirstCol - 1) * ITEM_ROW_GAP
    const itemBlockW = cols * ITEM_W + Math.max(0, cols - 1) * ITEM_COL_GAP

    // Turn node: horizontal spine
    const turnBlockH = Math.max(TURN_H, itemBlockH)
    const turnY = CANVAS_TOP + (turnBlockH - TURN_H) / 2
    result.push({
      ...turn,
      position: { x: curX, y: turnY },
      targetPosition: Position.Left,
      sourcePosition: Position.Right,
    })

    // Item nodes: to the right
    const itemsStartX = curX + TURN_W + ITEMS_LEFT_MARGIN
    const itemsStartY = CANVAS_TOP + (turnBlockH - itemBlockH) / 2  // center within turn block

    children.forEach((item, idx) => {
      const col = Math.floor(idx / ITEMS_PER_COL)
      const row = idx % ITEMS_PER_COL
      result.push({
        ...item,
        targetPosition: Position.Left,
        sourcePosition: Position.Right,
        position: {
          x: itemsStartX + col * (ITEM_W + ITEM_COL_GAP),
          y: itemsStartY + row * (ITEM_H + ITEM_ROW_GAP),
        },
      })
    })

    const turnBlockW = children.length > 0
      ? TURN_W + ITEMS_LEFT_MARGIN + itemBlockW
      : TURN_W
    curX += turnBlockW + TURN_GAP
  }

  return result
}

// ---------------------------------------------------------------------------
// Keep the dagre-based layout as a fallback for tests
// ---------------------------------------------------------------------------
import dagre from 'dagre'

const DAGRE_TURN_W = 220
const DAGRE_TURN_H = 64
const DAGRE_EVENT_W = 200
const DAGRE_EVENT_H = 48

export function applyDagreLayout(
  nodes: Node<GraphNodeData>[],
  edges: Edge[]
): Node<GraphNodeData>[] {
  const g = new dagre.graphlib.Graph()
  g.setDefaultEdgeLabel(() => ({}))
  g.setGraph({ rankdir: 'TB', nodesep: 40, ranksep: 60 })

  for (const node of nodes) {
    const w = node.type === 'turnNode' ? DAGRE_TURN_W : DAGRE_EVENT_W
    const h = node.type === 'turnNode' ? DAGRE_TURN_H : DAGRE_EVENT_H
    g.setNode(node.id, { width: w, height: h })
  }
  for (const edge of edges) {
    g.setEdge(edge.source, edge.target)
  }

  dagre.layout(g)

  return nodes.map(node => {
    const pos = g.node(node.id)
    const w = node.type === 'turnNode' ? DAGRE_TURN_W : DAGRE_EVENT_W
    const h = node.type === 'turnNode' ? DAGRE_TURN_H : DAGRE_EVENT_H
    return {
      ...node,
      position: { x: pos.x - w / 2, y: pos.y - h / 2 },
    }
  })
}
