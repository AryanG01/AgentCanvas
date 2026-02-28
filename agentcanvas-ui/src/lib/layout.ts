import dagre from 'dagre'
import type { Node, Edge } from '@xyflow/react'
import type { GraphNodeData } from './types'

const TURN_W = 220
const TURN_H = 64
const EVENT_W = 200
const EVENT_H = 48

export function applyDagreLayout(
  nodes: Node<GraphNodeData>[],
  edges: Edge[]
): Node<GraphNodeData>[] {
  const g = new dagre.graphlib.Graph()
  g.setDefaultEdgeLabel(() => ({}))
  g.setGraph({ rankdir: 'TB', nodesep: 40, ranksep: 60 })

  for (const node of nodes) {
    const w = node.type === 'turnNode' ? TURN_W : EVENT_W
    const h = node.type === 'turnNode' ? TURN_H : EVENT_H
    g.setNode(node.id, { width: w, height: h })
  }
  for (const edge of edges) {
    g.setEdge(edge.source, edge.target)
  }

  dagre.layout(g)

  return nodes.map(node => {
    const pos = g.node(node.id)
    const w = node.type === 'turnNode' ? TURN_W : EVENT_W
    const h = node.type === 'turnNode' ? TURN_H : EVENT_H
    return {
      ...node,
      position: {
        x: pos.x - w / 2,
        y: pos.y - h / 2,
      },
    }
  })
}
