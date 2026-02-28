import React from 'react'
import { useGraphStore } from '../store/graphStore'
import type {
  CommandExecutionEvent,
  GraphNodeData,
  McpToolCallEvent,
  PatchApplyEvent,
  PlanUpdateEvent,
  SummaryNodeEvent,
  TurnStartedEvent,
} from '../lib/types'

export function EvidencePanel() {
  const selectedId = useGraphStore(s => s.selectedNodeId)
  const nodes = useGraphStore(s => s.nodes)
  const selectNode = useGraphStore(s => s.selectNode)

  const node = nodes.find(n => n.id === selectedId)
  if (!node) return null

  const { rawEvent, kind, label, aggregatedCommands } = node.data as GraphNodeData

  const kindColors: Record<string, string> = {
    turn:    'text-indigo-400',
    command: 'text-yellow-400',
    tool:    'text-blue-400',
    patch:   'text-green-400',
    plan:    'text-zinc-400',
    error:   'text-red-400',
    summary: 'text-fuchsia-400',
    phase:   'text-violet-400',
  }

  const kindLabels: Record<string, string> = {
    turn: 'TURN', command: 'CMD', tool: 'MCP TOOL',
    patch: 'FILE PATCH', plan: 'PLAN', error: 'ERROR', summary: 'SUMMARY', phase: 'PHASE',
  }

  return (
    <div className="fixed right-0 top-0 h-full w-[400px] bg-zinc-950 border-l border-zinc-800 shadow-2xl z-50 flex flex-col">
      {/* Header */}
      <div className="flex items-start justify-between px-4 py-3 border-b border-zinc-800 bg-zinc-900">
        <div className="flex-1 min-w-0 pr-3">
          <span className={`text-[10px] font-bold tracking-widest uppercase ${kindColors[kind] ?? 'text-zinc-400'}`}>
            {kindLabels[kind] ?? kind}
          </span>
          <p className="text-sm font-medium text-zinc-100 truncate mt-0.5">{label}</p>
        </div>
        <button
          onClick={() => selectNode(null)}
          className="flex-shrink-0 w-7 h-7 rounded-lg bg-zinc-800 hover:bg-zinc-700 flex items-center justify-center text-zinc-400 hover:text-white transition-colors text-base"
        >
          ×
        </button>
      </div>

      {/* Body */}
      <div className="flex-1 overflow-y-auto p-4 space-y-5">
        {kind === 'command' && aggregatedCommands ? (
          <CommandSummaryDetail commands={aggregatedCommands} />
        ) : kind === 'command' || kind === 'error' ? (
          <CommandDetail event={rawEvent as CommandExecutionEvent} />
        ) : kind === 'tool' ? (
          <McpDetail event={rawEvent as McpToolCallEvent} />
        ) : kind === 'patch' ? (
          <PatchDetail event={rawEvent as PatchApplyEvent} />
        ) : kind === 'plan' ? (
          <PlanDetail event={rawEvent as PlanUpdateEvent} />
        ) : kind === 'phase' ? (
          <PhaseDetail event={rawEvent as SummaryNodeEvent} />
        ) : kind === 'summary' ? (
          <SummaryDetail event={rawEvent as SummaryNodeEvent} />
        ) : kind === 'turn' ? (
          <TurnDetail event={rawEvent as TurnStartedEvent} />
        ) : (
          <Section title="Raw Event">
            <CodeBlock>{JSON.stringify(rawEvent, null, 2)}</CodeBlock>
          </Section>
        )}
      </div>
    </div>
  )
}

function CommandDetail({ event }: { event: CommandExecutionEvent }) {
  const isError = event.exitCode !== 0 && event.exitCode !== null
  return (
    <>
      <Section title="Command">
        <CodeBlock className="text-yellow-300">{event.cmd}</CodeBlock>
      </Section>
      <Section title="Exit Code">
        <span className={`inline-flex items-center gap-1.5 text-xs font-mono px-2 py-0.5 rounded ${isError ? 'bg-red-900/50 text-red-300' : 'bg-green-900/50 text-green-300'}`}>
          <span className={`h-1.5 w-1.5 rounded-full ${isError ? 'bg-red-400' : 'bg-green-400'}`} />
          {event.exitCode ?? 'running'}
        </span>
      </Section>
      {event.stdout && (
        <Section title="stdout">
          <CodeBlock className="text-green-300">{event.stdout}</CodeBlock>
        </Section>
      )}
      {event.stderr && (
        <Section title="stderr">
          <CodeBlock className="text-red-300">{event.stderr}</CodeBlock>
        </Section>
      )}
    </>
  )
}

function CommandSummaryDetail({ commands }: { commands: CommandExecutionEvent[] }) {
  return (
    <Section title={`${commands.length} Command${commands.length !== 1 ? 's' : ''} Executed`}>
      <div className="space-y-2">
        {commands.map((cmd, i) => (
          <div
            key={cmd.id ?? i}
            className="rounded-lg border border-zinc-800 bg-zinc-900 p-3"
          >
            <div className="flex items-center justify-between mb-1.5">
              <span className="text-[10px] font-mono text-zinc-500">#{i + 1}</span>
              <span className={`inline-flex items-center gap-1 text-[10px] font-mono px-1.5 py-0.5 rounded ${
                cmd.exitCode === 0 || cmd.exitCode === null
                  ? 'bg-green-900/50 text-green-300'
                  : 'bg-red-900/50 text-red-300'
              }`}>
                <span className={`h-1.5 w-1.5 rounded-full ${
                  cmd.exitCode === 0 || cmd.exitCode === null ? 'bg-green-400' : 'bg-red-400'
                }`} />
                {cmd.exitCode ?? 'ok'}
              </span>
            </div>
            <pre className="text-xs text-yellow-300 whitespace-pre-wrap break-words leading-relaxed">
              {cmd.cmd}
            </pre>
            {cmd.stdout && (
              <pre className="text-[11px] text-zinc-500 whitespace-pre-wrap break-words mt-1.5 max-h-24 overflow-y-auto">
                {cmd.stdout.slice(0, 500)}{cmd.stdout.length > 500 ? '…' : ''}
              </pre>
            )}
          </div>
        ))}
      </div>
    </Section>
  )
}

function McpDetail({ event }: { event: McpToolCallEvent }) {
  return (
    <>
      <Section title="Tool">
        <span className="inline-block text-xs font-mono bg-blue-900/50 text-blue-300 px-2 py-0.5 rounded border border-blue-700/40">
          {event.toolName}
        </span>
      </Section>
      <Section title="Status">
        <span className={`text-xs font-mono ${event.status === 'success' ? 'text-green-400' : 'text-red-400'}`}>
          {event.status}
        </span>
      </Section>
      <Section title="Input">
        <CodeBlock>{JSON.stringify(event.input, null, 2)}</CodeBlock>
      </Section>
      <Section title="Output">
        <CodeBlock>{JSON.stringify(event.output, null, 2)}</CodeBlock>
      </Section>
    </>
  )
}

function PatchDetail({ event }: { event: PatchApplyEvent }) {
  const lines = event.diff.split('\n')
  return (
    <>
      <Section title="File">
        <span className="text-xs font-mono text-green-300 bg-green-900/30 px-2 py-0.5 rounded border border-green-700/30">
          {event.filename}
        </span>
      </Section>
      <Section title="Status">
        <span className={`text-xs font-mono ${event.status === 'success' ? 'text-green-400' : 'text-red-400'}`}>
          {event.status}
        </span>
      </Section>
      <Section title="Diff">
        <div className="rounded-lg overflow-hidden border border-zinc-700 text-xs font-mono">
          {lines.map((line, i) => {
            const color = line.startsWith('+') && !line.startsWith('+++')
              ? 'bg-green-900/30 text-green-300'
              : line.startsWith('-') && !line.startsWith('---')
              ? 'bg-red-900/30 text-red-300'
              : line.startsWith('@@')
              ? 'bg-blue-900/20 text-blue-400'
              : 'text-zinc-400'
            return (
              <div key={i} className={`px-3 py-0.5 whitespace-pre-wrap break-all ${color}`}>
                {line || ' '}
              </div>
            )
          })}
        </div>
      </Section>
    </>
  )
}

function PlanDetail({ event }: { event: PlanUpdateEvent }) {
  return (
    <Section title="Plan">
      <CodeBlock className="text-zinc-200">{event.text}</CodeBlock>
    </Section>
  )
}

function TurnDetail({ event }: { event: TurnStartedEvent }) {
  return (
    <>
      <Section title="Prompt">
        <p className="text-sm text-zinc-200 leading-relaxed">{event.userPrompt}</p>
      </Section>
      <Section title="Session ID">
        <span className="text-xs font-mono text-zinc-400">{event.sessionId}</span>
      </Section>
    </>
  )
}

function ActivityStats({ event }: { event: SummaryNodeEvent }) {
  const { commandsTotal, filePathsTotal, errorsTotal } = event.counts
  const hasErrors = errorsTotal > 0
  return (
    <div className="flex flex-wrap gap-1.5">
      {commandsTotal > 0 && (
        <span className={`inline-flex items-center gap-1 text-[11px] font-mono px-2 py-0.5 rounded ${hasErrors ? 'bg-yellow-900/40 text-yellow-300 border border-yellow-700/30' : 'bg-green-900/40 text-green-300 border border-green-700/30'}`}>
          ⌘ {commandsTotal}
        </span>
      )}
      {filePathsTotal > 0 && (
        <span className="inline-flex items-center gap-1 text-[11px] font-mono px-2 py-0.5 rounded bg-green-900/40 text-green-300 border border-green-700/30">
          ◇ {filePathsTotal} file{filePathsTotal !== 1 ? 's' : ''}
        </span>
      )}
      {errorsTotal > 0 && (
        <span className="inline-flex items-center gap-1 text-[11px] font-mono px-2 py-0.5 rounded bg-red-900/40 text-red-300 border border-red-700/30">
          ✕ {errorsTotal} error{errorsTotal !== 1 ? 's' : ''}
        </span>
      )}
      <span className="inline-flex items-center gap-1 text-[11px] font-mono px-2 py-0.5 rounded bg-fuchsia-900/30 text-fuchsia-300 border border-fuchsia-700/30">
        {event.brief.signal}
      </span>
    </div>
  )
}

function CommandList({ commands }: { commands: Array<{ command: string; exitCode: number | null }> }) {
  const [expanded, setExpanded] = React.useState(false)
  const visible = expanded ? commands : commands.slice(0, 5)
  return (
    <div className="rounded-lg border border-zinc-800 bg-zinc-900 p-2.5 space-y-1">
      {visible.map((cmd, i) => {
        const ok = cmd.exitCode === 0 || cmd.exitCode === null
        return (
          <div key={i} className="flex items-center gap-2 text-xs font-mono py-0.5">
            <span className="text-zinc-500 select-none">$</span>
            <span className="flex-1 text-yellow-300 truncate">{cmd.command}</span>
            <span className={`flex-shrink-0 h-1.5 w-1.5 rounded-full ${ok ? 'bg-green-400' : 'bg-red-400'}`} />
          </div>
        )
      })}
      {commands.length > 5 && !expanded && (
        <button
          onClick={() => setExpanded(true)}
          className="text-[10px] text-zinc-500 hover:text-zinc-300 pt-1"
        >
          +{commands.length - 5} more…
        </button>
      )}
    </div>
  )
}

function CompactLineage({ lineage }: { lineage: SummaryNodeEvent['lineage'] }) {
  const parts: string[] = []
  if (lineage.parentTurnId) parts.push(`↑ ${lineage.parentTurnId}`)
  if (lineage.childTurnId) parts.push(`→ ${lineage.childTurnId}`)
  if (lineage.forkedFromThreadId) parts.push(`⑂ ${lineage.forkedFromThreadId}`)
  if (lineage.startedAfterRollback) parts.push('↺ rollback')
  if (lineage.wasRolledBack) parts.push('⊘ rolled back')
  if (parts.length === 0) return null
  return (
    <Section title="Lineage">
      <span className="text-xs font-mono text-zinc-400">{parts.join('  ')}</span>
    </Section>
  )
}

function SummaryDetail({ event }: { event: SummaryNodeEvent }) {
  const [showAgent, setShowAgent] = React.useState(false)
  const summaryText = event.summaryText?.trim()
  const agentMsg = event.brief.agentMessage?.trim()
  const commands = event.evidence.commands ?? []
  const filePaths = event.evidence.filePaths ?? []
  const errors = event.evidence.errors ?? []

  return (
    <>
      {/* Summary prose */}
      {summaryText && (
        <Section title="Summary">
          <p className="text-sm text-fuchsia-200 leading-relaxed">{summaryText}</p>
        </Section>
      )}

      {/* Activity stats */}
      <Section title="Activity">
        <ActivityStats event={event} />
      </Section>

      {/* Commands */}
      {commands.length > 0 && (
        <Section title="Commands">
          <CommandList commands={commands} />
        </Section>
      )}

      {/* Files */}
      {filePaths.length > 0 && (
        <Section title="Files">
          <div className="rounded-lg border border-zinc-800 bg-zinc-900 p-2.5 space-y-1">
            {filePaths.map((fp, i) => (
              <div key={i} className="text-xs font-mono text-green-300 truncate py-0.5">{fp}</div>
            ))}
          </div>
        </Section>
      )}

      {/* Errors */}
      {errors.length > 0 && (
        <Section title="Errors">
          <div className="space-y-1">
            {errors.map((err, i) => (
              <div key={i} className="text-xs font-mono text-red-300 bg-red-900/20 rounded px-2 py-1 border border-red-800/30">
                {err}
              </div>
            ))}
          </div>
        </Section>
      )}

      {/* Agent message (collapsible) */}
      {agentMsg && (
        <div>
          <button
            onClick={() => setShowAgent(!showAgent)}
            className="text-[10px] font-bold text-zinc-500 uppercase tracking-widest mb-1 hover:text-zinc-300 flex items-center gap-1"
          >
            <span className="text-[8px]">{showAgent ? '▼' : '▶'}</span>
            Agent Message
          </button>
          {showAgent && (
            <p className="text-xs text-zinc-400 leading-relaxed bg-zinc-900 rounded-lg p-3 border border-zinc-800">
              {agentMsg}
            </p>
          )}
        </div>
      )}

      {/* Lineage */}
      <CompactLineage lineage={event.lineage} />
    </>
  )
}

function PhaseDetail({ event }: { event: SummaryNodeEvent }) {
  const selectNode = useGraphStore(s => s.selectNode)
  const nodes = useGraphStore(s => s.nodes)
  const childTurnIds = event.lineage.childTurnIds.length > 0
    ? event.lineage.childTurnIds
    : event.evidence.childTurnIds
  const summaryText = event.summaryText?.trim()
  const errors = event.evidence.errors ?? []

  return (
    <>
      {/* Phase summary prose */}
      {summaryText && (
        <Section title="Phase Summary">
          <p className="text-sm text-violet-200 leading-relaxed">{summaryText}</p>
        </Section>
      )}

      {/* Aggregated stats */}
      <Section title="Activity">
        <ActivityStats event={event} />
      </Section>

      {/* Turn list */}
      {childTurnIds.length > 0 && (
        <Section title={`${childTurnIds.length} Turn${childTurnIds.length !== 1 ? 's' : ''}`}>
          <div className="space-y-1.5">
            {childTurnIds.map((turnId) => {
              const turnNode = nodes.find(n => n.id === `turn-${turnId}`)
              const summaryNode = nodes.find(n => n.id === `summary-${turnId}`)
              const label = summaryNode?.data.label ?? turnNode?.data.label ?? turnId
              return (
                <button
                  key={turnId}
                  onClick={() => selectNode(`turn-${turnId}`)}
                  className="w-full text-left rounded-lg border border-zinc-800 bg-zinc-900 px-3 py-2 hover:border-zinc-600 transition-colors"
                >
                  <div className="flex items-center justify-between mb-0.5">
                    <span className="text-[10px] font-mono text-zinc-500">{turnId}</span>
                    <span className={`text-[10px] font-mono ${turnNode?.data.status === 'error' ? 'text-red-400' : 'text-green-400'}`}>
                      {turnNode?.data.status ?? 'unknown'}
                    </span>
                  </div>
                  <p className="text-xs text-zinc-200 truncate">{label}</p>
                </button>
              )
            })}
          </div>
        </Section>
      )}

      {/* Errors */}
      {errors.length > 0 && (
        <Section title="Errors">
          <div className="space-y-1">
            {errors.map((err, i) => (
              <div key={i} className="text-xs font-mono text-red-300 bg-red-900/20 rounded px-2 py-1 border border-red-800/30">
                {err}
              </div>
            ))}
          </div>
        </Section>
      )}

      {/* Lineage */}
      <CompactLineage lineage={event.lineage} />
    </>
  )
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div>
      <div className="text-[10px] font-bold text-zinc-500 uppercase tracking-widest mb-2">
        {title}
      </div>
      {children}
    </div>
  )
}

function CodeBlock({ children, className = 'text-zinc-300' }: { children: string; className?: string }) {
  return (
    <pre className={`text-xs leading-relaxed whitespace-pre-wrap break-words bg-zinc-900 rounded-lg p-3 border border-zinc-800 overflow-x-auto ${className}`}>
      {children}
    </pre>
  )
}
