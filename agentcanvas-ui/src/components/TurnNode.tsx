import { memo } from 'react'
import { Handle, Position, type NodeProps } from '@xyflow/react'
import type { GraphNodeData } from '../lib/types'

const statusConfig: Record<string, { dot: string; ring: string; label: string }> = {
  running:   { dot: 'bg-yellow-400 animate-pulse', ring: 'border-yellow-500/40',  label: 'In Progress' },
  success:   { dot: 'bg-green-400',                ring: 'border-green-500/40',   label: 'Done' },
  error:     { dot: 'bg-red-400',                  ring: 'border-red-500/40',     label: 'Failed' },
  cancelled: { dot: 'bg-zinc-400',                 ring: 'border-zinc-500/40',    label: 'Cancelled' },
}

export const TurnNode = memo(({ data, selected }: NodeProps<GraphNodeData>) => {
  const cfg = statusConfig[data.status ?? 'running']
  return (
    <div
      className={`
        w-[240px] rounded-xl border-2 px-4 py-3 shadow-xl
        bg-gradient-to-b from-zinc-800 to-zinc-900 text-white
        transition-all duration-150
        ${selected ? 'border-indigo-400 shadow-indigo-500/20 shadow-2xl' : `${cfg.ring} border`}
      `}
    >
      <Handle
        type="target"
        position={Position.Top}
        className="!bg-zinc-600 !border-zinc-500 !w-2 !h-2"
      />

      {/* Header row */}
      <div className="flex items-center justify-between mb-2">
        <div className="flex items-center gap-1.5">
          <span className="text-[10px] font-bold text-indigo-400 uppercase tracking-widest">
            Turn
          </span>
        </div>
        <div className="flex items-center gap-1.5">
          <span className={`h-2 w-2 rounded-full ${cfg.dot}`} />
          <span className="text-[10px] text-zinc-400">{cfg.label}</span>
        </div>
      </div>

      {/* Prompt text */}
      <p className="text-sm leading-snug text-zinc-100 line-clamp-3 font-medium">
        {data.label || <span className="text-zinc-500 italic">loading…</span>}
      </p>

      <Handle
        type="source"
        position={Position.Bottom}
        className="!bg-zinc-600 !border-zinc-500 !w-2 !h-2"
      />
    </div>
  )
})

TurnNode.displayName = 'TurnNode'
