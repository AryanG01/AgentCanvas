import { memo } from 'react'
import { Handle, Position, type NodeProps } from '@xyflow/react'
import type { GraphNodeData, NodeKind } from '../lib/types'

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
  output: {
    icon: '▣',
    badge: 'bg-emerald-900/60 text-emerald-300 border border-emerald-700/40',
    badgeText: 'OUT',
    border: 'border-emerald-700/30',
    iconBg: 'bg-emerald-900/50 text-emerald-300',
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
}

export const EventNode = memo(({ data, selected }: NodeProps<GraphNodeData>) => {
  const cfg = kindConfig[data.kind]
  return (
    <div
      className={`
        w-[210px] rounded-lg border px-3 py-2 shadow-lg
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
          <p className="text-xs text-zinc-200 leading-tight truncate font-mono">
            {data.label}
          </p>
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
