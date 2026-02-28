import { memo } from 'react'
import { Handle, Position, type NodeProps } from '@xyflow/react'
import type { GraphNodeData, NodeKind } from '../lib/types'

const kindStyle: Record<NodeKind, { dot: string; badge: string; label: string }> = {
  command: { dot: 'bg-yellow-400', badge: 'bg-yellow-900 text-yellow-200', label: 'CMD' },
  tool:    { dot: 'bg-blue-400',   badge: 'bg-blue-900 text-blue-200',     label: 'MCP' },
  patch:   { dot: 'bg-green-400',  badge: 'bg-green-900 text-green-200',   label: 'PATCH' },
  plan:    { dot: 'bg-zinc-400',   badge: 'bg-zinc-700 text-zinc-200',     label: 'PLAN' },
  error:   { dot: 'bg-red-500',    badge: 'bg-red-900 text-red-200',       label: 'ERR' },
  turn:    { dot: 'bg-zinc-400',   badge: 'bg-zinc-700 text-zinc-200',     label: 'TURN' },
}

export const EventNode = memo(({ data, selected }: NodeProps<GraphNodeData>) => {
  const style = kindStyle[data.kind]
  return (
    <div
      className={`
        w-[200px] rounded-md border px-2.5 py-1.5 shadow-sm bg-zinc-800 text-white
        ${selected ? 'border-blue-400' : 'border-zinc-600'}
      `}
    >
      <Handle type="target" position={Position.Top} className="!bg-zinc-600" />
      <div className="flex items-center gap-2">
        <span className={`h-2 w-2 rounded-full flex-shrink-0 ${style.dot}`} />
        <span className={`text-[10px] font-bold px-1 rounded ${style.badge}`}>{style.label}</span>
        <span className="text-xs text-zinc-300 truncate">{data.label}</span>
      </div>
      <Handle type="source" position={Position.Bottom} className="!bg-zinc-600" />
    </div>
  )
})

EventNode.displayName = 'EventNode'
