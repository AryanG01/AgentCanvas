import { describe, it, expect } from 'vitest'
import { buildGraph } from './eventToGraph'
import type { AppEvent } from './types'

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
  it('creates a turn node from TurnStarted', () => {
    const { nodes } = buildGraph([turnStarted])
    expect(nodes).toHaveLength(1)
    expect(nodes[0].data.kind).toBe('turn')
    expect(nodes[0].data.label).toContain('List files')
  })

  it('creates a child command node', () => {
    const { nodes, edges } = buildGraph([turnStarted, cmdEvent])
    expect(nodes).toHaveLength(2)
    const cmdNode = nodes.find(n => n.data.kind === 'command')
    expect(cmdNode).toBeDefined()
    expect(edges).toHaveLength(1)
    expect(edges[0].source).toBe('turn-t1')
    expect(edges[0].target).toBe('event-e1')
  })

  it('marks non-zero exit as error kind', () => {
    const { nodes } = buildGraph([turnStarted, errorCmd])
    const errNode = nodes.find(n => n.id === 'event-e2')
    expect(errNode?.data.kind).toBe('error')
  })

  it('returns no nodes for empty events', () => {
    const { nodes, edges } = buildGraph([])
    expect(nodes).toHaveLength(0)
    expect(edges).toHaveLength(0)
  })
})
