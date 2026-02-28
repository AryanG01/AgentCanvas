import { useCallback } from 'react'
import {
  ReactFlow,
  Background,
  Controls,
  MiniMap,
  MarkerType,
  BackgroundVariant,
  type NodeMouseHandler,
  type DefaultEdgeOptions,
} from '@xyflow/react'
import { useGraphStore } from '../store/graphStore'
import { TurnNode } from './TurnNode'
import { EventNode } from './EventNode'
import type { GraphNodeData } from '../lib/types'
import type { Node } from '@xyflow/react'

// Defined at module scope — recreating on each render causes node remounting
const nodeTypes = {
  turnNode: TurnNode,
  eventNode: EventNode,
}

const defaultEdgeOptions: DefaultEdgeOptions = {
  type: 'smoothstep',
  animated: false,
  style: { stroke: '#52525b', strokeWidth: 1.5 },
  markerEnd: {
    type: MarkerType.ArrowClosed,
    width: 14,
    height: 14,
    color: '#52525b',
  },
}

export function GraphCanvas() {
  const rawNodes = useGraphStore(s => s.nodes)
  const edges = useGraphStore(s => s.edges)
  const selectedId = useGraphStore(s => s.selectedNodeId)
  const selectNode = useGraphStore(s => s.selectNode)
  const searchQuery = useGraphStore(s => s.searchQuery)

  // Apply dim + search highlighting + running-turn edge animation
  const nodes = rawNodes.map(n => ({
    ...n,
    selected: n.id === selectedId,
    style: searchQuery
      ? n.data.label.toLowerCase().includes(searchQuery.toLowerCase())
        ? { filter: 'drop-shadow(0 0 6px rgba(99,102,241,0.6))' }
        : { opacity: 0.15 }
      : {},
  }))

  // Animate edges whose source turn is still running
  const runningTurnIds = new Set(
    rawNodes
      .filter(n => n.data.kind === 'turn' && n.data.status === 'running')
      .map(n => n.id)
  )
  const styledEdges = edges.map(e => ({
    ...e,
    animated: runningTurnIds.has(e.source),
    style: {
      stroke: runningTurnIds.has(e.source) ? '#6366f1' : '#52525b',
      strokeWidth: runningTurnIds.has(e.source) ? 2 : 1.5,
    },
    markerEnd: {
      type: MarkerType.ArrowClosed,
      width: 14,
      height: 14,
      color: runningTurnIds.has(e.source) ? '#6366f1' : '#52525b',
    },
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
        edges={styledEdges}
        nodeTypes={nodeTypes}
        defaultEdgeOptions={defaultEdgeOptions}
        onNodeClick={onNodeClick}
        fitView
        fitViewOptions={{ padding: 0.25 }}
        minZoom={0.2}
        proOptions={{ hideAttribution: true }}
      >
        <Background
          variant={BackgroundVariant.Dots}
          color="#27272a"
          gap={20}
          size={1.5}
        />
        <Controls
          showInteractive={false}
          className="!bg-zinc-900 !border-zinc-700 !shadow-xl [&>button]:!bg-zinc-900 [&>button]:!border-zinc-700 [&>button]:!text-zinc-400 [&>button:hover]:!bg-zinc-800 [&>button:hover]:!text-white"
        />
        <MiniMap
          className="!bg-zinc-900 !border-zinc-700 !rounded-lg"
          maskColor="rgba(0,0,0,0.6)"
          nodeColor={n => {
            if (n.type === 'turnNode') return '#6366f1'
            const kind = (n.data as GraphNodeData)?.kind
            if (kind === 'error') return '#ef4444'
            if (kind === 'tool') return '#3b82f6'
            if (kind === 'patch') return '#22c55e'
            if (kind === 'plan') return '#a1a1aa'
            return '#eab308'
          }}
        />
      </ReactFlow>
    </div>
  )
}
