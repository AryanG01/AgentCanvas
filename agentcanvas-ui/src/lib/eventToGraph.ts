import type { Node, Edge } from '@xyflow/react'
import type { AppEvent, CommandExecutionEvent, GraphNodeData, SummaryNodeEvent } from './types'

type GraphNode = Node<GraphNodeData>

export interface GraphResult {
  nodes: GraphNode[]
  edges: Edge[]
  turnOrder: string[]  // ordered list of turnIds for sequential flow edges
}

const SUMMARY_LABEL_MAX = 80

/**
 * Extract a short, human-readable label from a SummaryNodeEvent.
 *
 * Priority:
 * 1. Agent message from brief (most meaningful for display)
 * 2. Short extract from summaryText (skip verbose "Outcome:/Evidence:/Lineage:" boilerplate)
 * 3. Primary evidence (command, file, or error)
 * 4. Status-based fallback
 */
function extractSummaryLabel(event: SummaryNodeEvent): string {
  // 1. Agent message is the best short label
  const agentMsg = event.brief.agentMessage?.trim()
  if (agentMsg) return agentMsg.slice(0, SUMMARY_LABEL_MAX)

  // 2. Try to extract the useful part from summaryText
  const text = event.summaryText?.trim() ?? ''
  if (text) {
    // If it starts with "Outcome:" it's a compact summary — extract the Agent: part if present
    const agentMatch = text.match(/Agent:\s*(.+?)(?:\.\s*Evidence:|$)/i)
    if (agentMatch?.[1]?.trim()) return agentMatch[1].trim().slice(0, SUMMARY_LABEL_MAX)

    // Extract the Outcome: part as fallback
    const outcomeMatch = text.match(/Outcome:\s*(.+?)(?:\.\s*Evidence:|$)/i)
    if (outcomeMatch?.[1]?.trim()) return outcomeMatch[1].trim().slice(0, SUMMARY_LABEL_MAX)

    // Not a structured format — use it directly if short enough
    if (!text.includes('Evidence:') && !text.includes('Lineage:')) {
      return text.slice(0, SUMMARY_LABEL_MAX)
    }
  }

  // 3. Primary evidence
  if (event.brief.primaryError) return `Error: ${event.brief.primaryError.slice(0, 60)}`
  if (event.brief.primaryCommand) return `$ ${event.brief.primaryCommand.slice(0, 60)}`
  if (event.brief.primaryFilePath) return event.brief.primaryFilePath.slice(0, 60)

  // 4. Status fallback — human-readable
  const signalLabels: Record<string, string> = {
    error: 'Failed',
    code_changes: 'Code changes',
    execution: 'Commands executed',
    status_only: event.status === 'completed' ? 'Completed' : event.status ?? 'Done',
  }
  return signalLabels[event.brief.signal] ?? 'Summary'
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
    } else if (event.type === 'SummaryNode') {
      const summaryStatus =
        event.status === 'rolled_back'
          ? 'cancelled'
          : event.status === 'in_progress'
            ? 'running'
            : event.status === 'failed' || event.brief.signal === 'error' || event.status === 'error'
              ? 'error'
              : 'success'
      if (event.nodeType === 'phase') {
        const phaseNodeId = `phase-${event.id}`
        const phaseLabel = event.summaryText.trim()
          || `Phase (${event.lineage.childTurnIds.length} turn${event.lineage.childTurnIds.length === 1 ? '' : 's'})`
        const existingPhaseNode = nodes.find(n => n.id === phaseNodeId)
        if (existingPhaseNode) {
          existingPhaseNode.data = {
            ...existingPhaseNode.data,
            label: phaseLabel,
            status: summaryStatus,
            rawEvent: event,
          }
        } else {
          nodes.push({
            id: phaseNodeId,
            type: 'eventNode',
            position: { x: 0, y: 0 },
            data: {
              kind: 'phase',
              label: phaseLabel,
              status: summaryStatus,
              turnId: event.turnId,
              rawEvent: event,
            },
          })
        }
        const phaseEdgeSources = event.lineage.childTurnIds.length > 0
          ? event.lineage.childTurnIds
          : [event.turnId]
        for (const phaseTurnId of phaseEdgeSources) {
          const edgeId = `detail-${phaseNodeId}-${phaseTurnId}`
          if (!edges.find(e => e.id === edgeId)) {
            edges.push({
              id: edgeId,
              source: `turn-${phaseTurnId}`,
              target: phaseNodeId,
              data: { kind: 'detail' },
            })
          }
        }
      } else {
        const turnNode = nodes.find(n => n.id === `turn-${event.turnId}`)
        if (turnNode) {
          const extractedLabel = extractSummaryLabel(event)
          if (turnNode.data.label === '(typing…)' && extractedLabel) {
            turnNode.data = { ...turnNode.data, label: extractedLabel }
          }
          if (event.status === 'rolled_back') {
            turnNode.data = { ...turnNode.data, status: 'cancelled' }
          }
        }

        const summaryNodeId = `summary-${event.turnId}`
        const summaryLabel = extractSummaryLabel(event)
        const existingSummaryNode = nodes.find(n => n.id === summaryNodeId)
        if (existingSummaryNode) {
          existingSummaryNode.data = {
            ...existingSummaryNode.data,
            label: summaryLabel,
            status: summaryStatus,
            rawEvent: event,
          }
        } else {
          nodes.push({
            id: summaryNodeId,
            type: 'eventNode',
            position: { x: 0, y: 0 },
            data: {
              kind: 'summary',
              label: summaryLabel,
              status: summaryStatus,
              turnId: event.turnId,
              rawEvent: event,
            },
          })
          edges.push({
            id: `detail-${summaryNodeId}`,
            source: `turn-${event.turnId}`,
            target: summaryNodeId,
            data: { kind: 'detail' },
          })
        }
      }
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
