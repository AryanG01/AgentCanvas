import { memo } from 'react'
import { Handle, Position, type Node, type NodeProps } from '@xyflow/react'
import type { GraphNodeData } from '../lib/types'

type GraphNode = Node<GraphNodeData>

export const SessionNode = memo(({ data, selected }: NodeProps<GraphNode>) => {
  return (
    <div
      className={`
        w-[260px] rounded-2xl border-2 px-4 py-2.5 shadow-2xl
        bg-gradient-to-br from-indigo-950 to-zinc-900 text-white
        ${selected ? 'border-indigo-300' : 'border-indigo-700/60'}
      `}
    >
      <div className="flex items-center gap-2.5">
        <div className="flex items-center gap-1">
          <span className="h-2 w-2 rounded-full bg-green-400 animate-pulse" />
          <span className="h-2 w-2 rounded-full bg-indigo-400" />
          <span className="h-2 w-2 rounded-full bg-zinc-500" />
        </div>
        <div className="flex-1 min-w-0">
          <div className="text-[9px] font-bold text-indigo-400 uppercase tracking-widest">
            Codex Session
          </div>
          <div className="text-xs text-zinc-300 font-mono truncate mt-0.5">
            {data.label.length > 20 ? `…${data.label.slice(-18)}` : data.label}
          </div>
        </div>
      </div>
      <Handle
        type="source"
        position={Position.Right}
        className="!bg-indigo-500 !border-indigo-400 !w-2 !h-2"
      />
    </div>
  )
})

SessionNode.displayName = 'SessionNode'
