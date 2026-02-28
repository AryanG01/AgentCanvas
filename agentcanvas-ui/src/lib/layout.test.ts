import { describe, it, expect } from 'vitest'
import { applyConversationLayout, applyDagreLayout } from './layout'
import type { Node, Edge } from '@xyflow/react'
import type { GraphNodeData } from './types'

describe('applyConversationLayout', () => {
  it('lays out turns left-to-right and keeps detail nodes to the right of their turn', () => {
    const rawEvent: GraphNodeData['rawEvent'] = {
      type: 'ThreadStarted',
      threadId: 'thread-1',
      ts: 0,
    }

    const nodes: Node<GraphNodeData>[] = [
      {
        id: 'session-s1',
        type: 'sessionNode',
        position: { x: 0, y: 0 },
        data: { kind: 'session', label: 's1', turnId: '', rawEvent },
      },
      {
        id: 'turn-t1',
        type: 'turnNode',
        position: { x: 0, y: 0 },
        data: { kind: 'turn', label: 'turn 1', turnId: 't1', rawEvent },
      },
      {
        id: 'turn-t2',
        type: 'turnNode',
        position: { x: 0, y: 0 },
        data: { kind: 'turn', label: 'turn 2', turnId: 't2', rawEvent },
      },
      {
        id: 'summary-t1',
        type: 'eventNode',
        position: { x: 0, y: 0 },
        data: { kind: 'summary', label: 'summary', turnId: 't1', rawEvent },
      },
      {
        id: 'cmds-t1',
        type: 'eventNode',
        position: { x: 0, y: 0 },
        data: { kind: 'command', label: 'command', turnId: 't1', rawEvent },
      },
    ]

    const result = applyConversationLayout(nodes, [])
    const session = result.find(n => n.id === 'session-s1')!.position
    const turn1 = result.find(n => n.id === 'turn-t1')!.position
    const turn2 = result.find(n => n.id === 'turn-t2')!.position
    const summary = result.find(n => n.id === 'summary-t1')!.position
    const command = result.find(n => n.id === 'cmds-t1')!.position

    expect(turn1.x).toBeGreaterThan(session.x)
    expect(turn2.x).toBeGreaterThan(turn1.x)
    expect(summary.x).toBeGreaterThan(turn1.x)
    expect(command.x).toBeGreaterThan(turn1.x)
    expect(summary.y).toBeLessThan(command.y)
  })
})

describe('applyDagreLayout', () => {
  it('assigns non-zero positions to all nodes', () => {
    const rawEvent: GraphNodeData['rawEvent'] = {
      type: 'ThreadStarted',
      threadId: 'thread-2',
      ts: 0,
    }

    const nodes: Node<GraphNodeData>[] = [
      {
        id: 'a',
        type: 'turnNode',
        position: { x: 0, y: 0 },
        data: { kind: 'turn', label: 'A', turnId: 'a', rawEvent },
      },
      {
        id: 'b',
        type: 'eventNode',
        position: { x: 0, y: 0 },
        data: { kind: 'command', label: 'B', turnId: 'a', rawEvent },
      },
    ]
    const edges: Edge[] = [{ id: 'e1', source: 'a', target: 'b' }]
    const result = applyDagreLayout(nodes, edges)
    const posA = result.find(n => n.id === 'a')!.position
    const posB = result.find(n => n.id === 'b')!.position
    expect(posB.y).toBeGreaterThan(posA.y) // B is below A in TB layout
  })
})
