import type { Node, Edge } from '@xyflow/react'
import type { GraphNodeData } from './types'

// Node dimensions
const SESSION_W = 260
const SESSION_H = 56
const TURN_W = 240
const TURN_H = 90    // taller to show prompt
const ITEM_W = 210
const ITEM_H = 52
const ITEM_ROW_GAP = 12   // vertical gap between items in same column
const ITEM_COL_GAP = 24   // horizontal gap between item columns
const TURN_GAP = 48       // vertical gap between turns
const ITEMS_LEFT_MARGIN = 60  // gap from right edge of turn to first item column
const SESSION_MARGIN_BOTTOM = 48

// How many items per column before wrapping to a new column
const ITEMS_PER_COL = 4

/**
 * Custom conversation-flow layout:
 *   [Session]
 *      |
 *   [Turn 1] ──── [CMD] [MCP]
 *      |          [PATCH][ERR]
 *   [Turn 2] ──── [MCP]
 *      |          [PLAN]
 *   [Turn 3] ──── ...
 *
 * Turns form a vertical spine at x=0.
 * Items branch to the right of their parent turn.
 */
export function applyConversationLayout(
  nodes: Node<GraphNodeData>[],
  _edges: Edge[]
): Node<GraphNodeData>[] {
  const sessionNode = nodes.find(n => n.data.kind === 'session')
  const turnNodes = nodes.filter(n => n.data.kind === 'turn')
  const itemNodes = nodes.filter(n => n.data.kind !== 'session' && n.data.kind !== 'turn')

  const result: Node<GraphNodeData>[] = []

  // Session at top-center of the turn spine
  if (sessionNode) {
    result.push({ ...sessionNode, position: { x: 0, y: 0 } })
  }

  let curY = (sessionNode ? SESSION_H + SESSION_MARGIN_BOTTOM : 0)

  for (const turn of turnNodes) {
    const children = itemNodes.filter(n => n.data.turnId === turn.data.turnId)

    // Calculate how much height the item block needs
    const cols = Math.ceil(children.length / ITEMS_PER_COL)
    const rowsInFirstCol = Math.min(children.length, ITEMS_PER_COL)
    const itemBlockH = rowsInFirstCol * ITEM_H + Math.max(0, rowsInFirstCol - 1) * ITEM_ROW_GAP

    // Turn node: left spine
    const turnH = Math.max(TURN_H, itemBlockH)
    result.push({ ...turn, position: { x: 0, y: curY } })

    // Item nodes: to the right
    const itemsStartX = TURN_W + ITEMS_LEFT_MARGIN
    const itemsStartY = curY + (turnH - itemBlockH) / 2  // vertically center the item block

    children.forEach((item, idx) => {
      const col = Math.floor(idx / ITEMS_PER_COL)
      const row = idx % ITEMS_PER_COL
      result.push({
        ...item,
        position: {
          x: itemsStartX + col * (ITEM_W + ITEM_COL_GAP),
          y: itemsStartY + row * (ITEM_H + ITEM_ROW_GAP),
        },
      })
    })

    // Width of item columns
    const itemBlockW = cols * ITEM_W + Math.max(0, cols - 1) * ITEM_COL_GAP

    curY += turnH + TURN_GAP

    // Silence unused var warning (itemBlockW used only for future minimap sizing)
    void itemBlockW
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
