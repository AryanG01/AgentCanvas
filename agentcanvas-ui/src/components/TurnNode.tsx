import { memo } from 'react'
import { Handle, Position, type NodeProps } from '@xyflow/react'
import type { GraphNodeData } from '../lib/types'

const statusColors: Record<string, string> = {
  running: 'bg-yellow-400',
  success: 'bg-green-400',
  error: 'bg-red-400',
  cancelled: 'bg-gray-400',
}

export const TurnNode = memo(({ data, selected }: NodeProps<GraphNodeData>) => {
  return (
    <div
      className={`
        w-[220px] rounded-lg border-2 px-3 py-2 shadow-md bg-zinc-900 text-white
        ${selected ? 'border-blue-400' : 'border-zinc-600'}
      `}
    >
      <Handle type="target" position={Position.Top} className="!bg-zinc-500" />
      <div className="flex items-center gap-2">
        <span className={`h-2.5 w-2.5 rounded-full flex-shrink-0 ${statusColors[data.status ?? 'running']}`} />
        <span className="text-xs font-semibold text-zinc-400 flex-shrink-0">Turn</span>
      </div>
      <p className="mt-1 text-sm leading-tight line-clamp-2 text-zinc-100">{data.label}</p>
      <Handle type="source" position={Position.Bottom} className="!bg-zinc-500" />
    </div>
  )
})

TurnNode.displayName = 'TurnNode'
