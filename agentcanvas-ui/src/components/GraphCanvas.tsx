import { useCallback } from 'react'
import {
  ReactFlow,
  Background,
  Controls,
  MiniMap,
  type NodeMouseHandler,
} from '@xyflow/react'
import { useGraphStore } from '../store/graphStore'
import { TurnNode } from './TurnNode'
import { EventNode } from './EventNode'
import type { GraphNodeData } from '../lib/types'
import type { Node } from '@xyflow/react'

// Must be defined at module scope — recreating on each render causes node remounting
const nodeTypes = {
  turnNode: TurnNode,
  eventNode: EventNode,
}

export function GraphCanvas() {
  const rawNodes = useGraphStore(s => s.nodes)
  const edges = useGraphStore(s => s.edges)
  const selectedId = useGraphStore(s => s.selectedNodeId)
  const selectNode = useGraphStore(s => s.selectNode)
  const searchQuery = useGraphStore(s => s.searchQuery)

  // Apply dim effect for non-matching search results
  const nodes = rawNodes.map(n => ({
    ...n,
    selected: n.id === selectedId,
    style: searchQuery
      ? n.data.label.toLowerCase().includes(searchQuery.toLowerCase())
        ? {}
        : { opacity: 0.15 }
      : {},
  }))

  const onNodeClick: NodeMouseHandler<Node<GraphNodeData>> = useCallback(
    (_evt, node) => {
      selectNode(node.id === selectedId ? null : node.id)
    },
    [selectedId, selectNode]
  )

  return (
    <div className="w-full h-full bg-zinc-950">
      <ReactFlow
        nodes={nodes}
        edges={edges}
        nodeTypes={nodeTypes}
        onNodeClick={onNodeClick}
        fitView
        fitViewOptions={{ padding: 0.2 }}
        proOptions={{ hideAttribution: true }}
      >
        <Background color="#3f3f46" gap={24} />
        <Controls className="!bg-zinc-800 !border-zinc-700 !shadow-lg" />
        <MiniMap
          className="!bg-zinc-900 !border-zinc-700"
          nodeColor={n => {
            if (n.type === 'turnNode') return '#6366f1'
            return '#52525b'
          }}
        />
      </ReactFlow>
    </div>
  )
}
