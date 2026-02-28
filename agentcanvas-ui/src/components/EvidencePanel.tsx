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
  }

  const kindLabels: Record<string, string> = {
    turn: 'TURN', command: 'CMD', tool: 'MCP TOOL',
    patch: 'FILE PATCH', plan: 'PLAN', error: 'ERROR', summary: 'SUMMARY',
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

function SummaryDetail({ event }: { event: SummaryNodeEvent }) {
  return (
    <>
      <Section title="Summary">
        <CodeBlock className="text-fuchsia-200">{event.summaryText || '(empty)'}</CodeBlock>
      </Section>

      <Section title="Signal">
        <span className="inline-flex items-center gap-1 text-xs font-mono bg-fuchsia-900/40 text-fuchsia-200 px-2 py-0.5 rounded border border-fuchsia-700/30">
          {event.brief.signal}
        </span>
      </Section>

      <Section title="Brief">
        <CodeBlock>
{JSON.stringify({
  agentMessage: event.brief.agentMessage,
  primaryCommand: event.brief.primaryCommand,
  primaryFilePath: event.brief.primaryFilePath,
  primaryError: event.brief.primaryError,
}, null, 2)}
        </CodeBlock>
      </Section>

      <Section title="Counts">
        <CodeBlock>{JSON.stringify(event.counts, null, 2)}</CodeBlock>
      </Section>

      <Section title="Digest">
        <CodeBlock>{JSON.stringify(event.digest, null, 2)}</CodeBlock>
      </Section>

      <Section title="Lineage">
        <CodeBlock>{JSON.stringify(event.lineage, null, 2)}</CodeBlock>
      </Section>

      <Section title="Evidence">
        <CodeBlock>{JSON.stringify(event.evidence, null, 2)}</CodeBlock>
      </Section>
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
