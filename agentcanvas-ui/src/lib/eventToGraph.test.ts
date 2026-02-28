import { describe, it, expect } from 'vitest'
import { buildGraph } from './eventToGraph'
import type { AppEvent } from './types'

const threadStarted: AppEvent = {
  type: 'ThreadStarted',
  threadId: 's1',
  ts: 900,
}

const turnStarted: AppEvent = {
  type: 'TurnStarted',
  turnId: 't1',
  sessionId: 's1',
  userPrompt: 'List files in /tmp',
  ts: 1000,
}

const cmdEvent: AppEvent = {
  type: 'CommandExecution',
  id: 'e1',
  turnId: 't1',
  cmd: 'ls /tmp',
  exitCode: 0,
  stdout: 'foo\nbar',
  stderr: '',
  ts: 1001,
}

const errorCmd: AppEvent = {
  type: 'CommandExecution',
  id: 'e2',
  turnId: 't1',
  cmd: 'cat /nonexistent',
  exitCode: 1,
  stdout: '',
  stderr: 'No such file',
  ts: 1002,
}

describe('buildGraph', () => {
  it('creates a session node from ThreadStarted', () => {
    const { nodes } = buildGraph([threadStarted])
    expect(nodes).toHaveLength(1)
    expect(nodes[0].data.kind).toBe('session')
    expect(nodes[0].id).toBe('session-s1')
  })

  it('creates a turn node from TurnStarted', () => {
    const { nodes } = buildGraph([threadStarted, turnStarted])
    const turnNode = nodes.find(n => n.data.kind === 'turn')
    expect(turnNode).toBeDefined()
    expect(turnNode?.data.label).toContain('List files')
  })

  it('creates a child command node with flow+detail edges', () => {
    const { nodes, edges } = buildGraph([threadStarted, turnStarted, cmdEvent])
    expect(nodes).toHaveLength(3)
    const cmdNode = nodes.find(n => n.data.kind === 'command')
    expect(cmdNode).toBeDefined()
    // session→turn flow edge + turn→cmd detail edge
    expect(edges).toHaveLength(2)
    const detailEdge = edges.find(e => e.target === 'event-e1')
    expect(detailEdge?.source).toBe('turn-t1')
  })

  it('marks non-zero exit as error kind', () => {
    const { nodes } = buildGraph([threadStarted, turnStarted, errorCmd])
    const errNode = nodes.find(n => n.id === 'event-e2')
    expect(errNode?.data.kind).toBe('error')
  })

  it('returns no nodes for empty events', () => {
    const { nodes, edges } = buildGraph([])
    expect(nodes).toHaveLength(0)
    expect(edges).toHaveLength(0)
  })
})
