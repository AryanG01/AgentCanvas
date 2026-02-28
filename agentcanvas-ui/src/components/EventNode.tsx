import { memo } from 'react'
import { Handle, Position, type Node, type NodeProps } from '@xyflow/react'
import type { GraphNodeData, NodeKind, SummaryNodeEvent } from '../lib/types'

type GraphNode = Node<GraphNodeData>

interface KindConfig {
  icon: string
  badge: string
  badgeText: string
  border: string
  iconBg: string
}

const kindConfig: Record<NodeKind, KindConfig> = {
  command: {
    icon: '$',
    badge: 'bg-yellow-900/60 text-yellow-300 border border-yellow-700/40',
    badgeText: 'CMD',
    border: 'border-yellow-700/30',
    iconBg: 'bg-yellow-900/50 text-yellow-300',
  },
  tool: {
    icon: '⚡',
    badge: 'bg-blue-900/60 text-blue-300 border border-blue-700/40',
    badgeText: 'MCP',
    border: 'border-blue-700/30',
    iconBg: 'bg-blue-900/50 text-blue-300',
  },
  patch: {
    icon: '✎',
    badge: 'bg-green-900/60 text-green-300 border border-green-700/40',
    badgeText: 'PATCH',
    border: 'border-green-700/30',
    iconBg: 'bg-green-900/50 text-green-300',
  },
  plan: {
    icon: '☰',
    badge: 'bg-zinc-700/60 text-zinc-300 border border-zinc-600/40',
    badgeText: 'PLAN',
    border: 'border-zinc-600/30',
    iconBg: 'bg-zinc-700/50 text-zinc-300',
  },
  summary: {
    icon: 'S',
    badge: 'bg-fuchsia-900/60 text-fuchsia-300 border border-fuchsia-700/40',
    badgeText: 'SUMMARY',
    border: 'border-fuchsia-700/40',
    iconBg: 'bg-fuchsia-900/50 text-fuchsia-300',
  },
  phase: {
    icon: 'Φ',
    badge: 'bg-violet-900/60 text-violet-300 border border-violet-700/40',
    badgeText: 'PHASE',
    border: 'border-violet-700/40',
    iconBg: 'bg-violet-900/50 text-violet-300',
  },
  error: {
    icon: '✕',
    badge: 'bg-red-900/60 text-red-300 border border-red-700/40',
    badgeText: 'ERR',
    border: 'border-red-700/50',
    iconBg: 'bg-red-900/50 text-red-300',
  },
  turn: {
    icon: '◆',
    badge: 'bg-indigo-900/60 text-indigo-300 border border-indigo-700/40',
    badgeText: 'TURN',
    border: 'border-indigo-700/30',
    iconBg: 'bg-indigo-900/50 text-indigo-300',
  },
  session: {
    icon: '◉',
    badge: 'bg-indigo-900/60 text-indigo-300 border border-indigo-700/40',
    badgeText: 'SESSION',
    border: 'border-indigo-700/30',
    iconBg: 'bg-indigo-900/50 text-indigo-300',
  },
}

function MiniStats({ event }: { event: SummaryNodeEvent }) {
  const { commandsTotal, filePathsTotal, errorsTotal } = event.counts
  if (commandsTotal === 0 && filePathsTotal === 0 && errorsTotal === 0) return null
  const parts: string[] = []
  if (commandsTotal > 0) parts.push(`⌘${commandsTotal}`)
  if (filePathsTotal > 0) parts.push(`◇${filePathsTotal}`)
  if (errorsTotal > 0) parts.push(`✕${errorsTotal}`)
  return (
    <span className="text-[10px] text-zinc-500 font-mono leading-none mt-0.5 block">
      {parts.join(' ')}
    </span>
  )
}

export const EventNode = memo(({ data, selected }: NodeProps<GraphNode>) => {
  const cfg = kindConfig[data.kind]
  const isSummaryLike = data.kind === 'summary' || data.kind === 'phase'
  const width = isSummaryLike ? 'w-[240px]' : 'w-[210px]'
  const summaryEvent = isSummaryLike ? (data.rawEvent as SummaryNodeEvent) : null

  return (
    <div
      className={`
        ${width} rounded-lg border px-3 py-2 shadow-lg
        bg-zinc-800/90 backdrop-blur-sm text-white
        transition-all duration-150
        ${selected ? 'border-indigo-400 shadow-indigo-500/20 shadow-xl' : cfg.border}
      `}
    >
      <Handle
        type="target"
        position={Position.Top}
        className="!bg-zinc-600 !border-zinc-500 !w-1.5 !h-1.5"
      />

      <div className="flex items-start gap-2">
        {/* Icon */}
        <div className={`flex-shrink-0 w-6 h-6 rounded flex items-center justify-center text-xs font-bold ${cfg.iconBg}`}>
          {cfg.icon}
        </div>

        {/* Content */}
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-1.5 mb-0.5">
            <span className={`text-[9px] font-bold px-1.5 py-0.5 rounded ${cfg.badge}`}>
              {cfg.badgeText}
            </span>
            {data.status === 'error' && data.kind !== 'error' && (
              <span className="text-[9px] text-red-400">failed</span>
            )}
          </div>
          <p className={`text-xs text-zinc-200 leading-tight font-mono ${isSummaryLike ? 'line-clamp-2' : 'truncate'}`}>
            {data.label}
          </p>
          {summaryEvent && <MiniStats event={summaryEvent} />}
        </div>
      </div>

      <Handle
        type="source"
        position={Position.Bottom}
        className="!bg-zinc-600 !border-zinc-500 !w-1.5 !h-1.5"
      />
    </div>
  )
})

EventNode.displayName = 'EventNode'
