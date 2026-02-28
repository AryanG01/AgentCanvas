import { describe, it, expect } from 'vitest'
import { applyDagreLayout } from './layout'
import type { Node, Edge } from '@xyflow/react'
import type { GraphNodeData } from './types'

describe('applyDagreLayout', () => {
  it('assigns non-zero positions to all nodes', () => {
    const nodes: Node<GraphNodeData>[] = [
      {
        id: 'a',
        type: 'turnNode',
        position: { x: 0, y: 0 },
        data: { kind: 'turn', label: 'A', turnId: 'a', rawEvent: {} as any },
      },
      {
        id: 'b',
        type: 'eventNode',
        position: { x: 0, y: 0 },
        data: { kind: 'command', label: 'B', turnId: 'a', rawEvent: {} as any },
      },
    ]
    const edges: Edge[] = [{ id: 'e1', source: 'a', target: 'b' }]
    const result = applyDagreLayout(nodes, edges)
    const posA = result.find(n => n.id === 'a')!.position
    const posB = result.find(n => n.id === 'b')!.position
    expect(posB.y).toBeGreaterThan(posA.y) // B is below A in TB layout
  })
})
