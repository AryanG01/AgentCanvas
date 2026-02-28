import { useGraphStore } from '../store/graphStore'
import type { CommandExecutionEvent, McpToolCallEvent, PatchApplyEvent, PlanUpdateEvent } from '../lib/types'

export function EvidencePanel() {
  const selectedId = useGraphStore(s => s.selectedNodeId)
  const nodes = useGraphStore(s => s.nodes)
  const selectNode = useGraphStore(s => s.selectNode)

  const node = nodes.find(n => n.id === selectedId)
  if (!node) return null

  const { rawEvent, kind, label } = node.data

  return (
    <div className="fixed right-0 top-0 h-full w-[380px] bg-zinc-900 border-l border-zinc-700 shadow-2xl z-50 flex flex-col">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-3 border-b border-zinc-700">
        <span className="text-sm font-semibold text-zinc-100 truncate pr-2">{label}</span>
        <button
          onClick={() => selectNode(null)}
          className="text-zinc-400 hover:text-white text-xl leading-none flex-shrink-0"
        >
          ×
        </button>
      </div>

      {/* Body */}
      <div className="flex-1 overflow-y-auto p-4 space-y-4 text-xs font-mono">
        {kind === 'command' || kind === 'error' ? (
          <CommandDetail event={rawEvent as CommandExecutionEvent} />
        ) : kind === 'tool' ? (
          <McpDetail event={rawEvent as McpToolCallEvent} />
        ) : kind === 'patch' ? (
          <PatchDetail event={rawEvent as PatchApplyEvent} />
        ) : kind === 'plan' ? (
          <PlanDetail event={rawEvent as PlanUpdateEvent} />
        ) : (
          <pre className="text-zinc-300 whitespace-pre-wrap break-words">
            {JSON.stringify(rawEvent, null, 2)}
          </pre>
        )}
      </div>
    </div>
  )
}

function CommandDetail({ event }: { event: CommandExecutionEvent }) {
  return (
    <>
      <Section title="Command">
        <pre className="text-yellow-300 whitespace-pre-wrap break-all">{event.cmd}</pre>
      </Section>
      <Section title={`Exit code: ${event.exitCode ?? 'running'}`}>
        <></>
      </Section>
      {event.stdout && (
        <Section title="stdout">
          <pre className="text-green-300 whitespace-pre-wrap break-words">{event.stdout}</pre>
        </Section>
      )}
      {event.stderr && (
        <Section title="stderr">
          <pre className="text-red-300 whitespace-pre-wrap break-words">{event.stderr}</pre>
        </Section>
      )}
    </>
  )
}

function McpDetail({ event }: { event: McpToolCallEvent }) {
  return (
    <>
      <Section title="Tool">
        <pre className="text-blue-300">{event.toolName}</pre>
      </Section>
      <Section title="Input">
        <pre className="text-zinc-300 whitespace-pre-wrap break-words">
          {JSON.stringify(event.input, null, 2)}
        </pre>
      </Section>
      <Section title="Output">
        <pre className="text-zinc-300 whitespace-pre-wrap break-words">
          {JSON.stringify(event.output, null, 2)}
        </pre>
      </Section>
    </>
  )
}

function PatchDetail({ event }: { event: PatchApplyEvent }) {
  return (
    <>
      <Section title={`File: ${event.filename}`}>
        <></>
      </Section>
      <Section title="Diff">
        <pre className="text-zinc-300 whitespace-pre-wrap break-words">{event.diff}</pre>
      </Section>
    </>
  )
}

function PlanDetail({ event }: { event: PlanUpdateEvent }) {
  return (
    <Section title="Plan">
      <pre className="text-zinc-300 whitespace-pre-wrap break-words">{event.text}</pre>
    </Section>
  )
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div>
      <div className="text-zinc-500 text-[10px] uppercase tracking-wide mb-1">{title}</div>
      {children}
    </div>
  )
}
