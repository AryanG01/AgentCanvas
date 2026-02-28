import { useCallback } from 'react'
import {
  ReactFlow,
  Background,
  Controls,
  MiniMap,
  MarkerType,
  BackgroundVariant,
  type NodeMouseHandler,
} from '@xyflow/react'
import { useGraphStore } from '../store/graphStore'
import { TurnNode } from './TurnNode'
import { EventNode } from './EventNode'
import { SessionNode } from './SessionNode'
import type { GraphNodeData, EdgeKind } from '../lib/types'
import type { Node } from '@xyflow/react'

const nodeTypes = {
  sessionNode: SessionNode,
  turnNode: TurnNode,
  eventNode: EventNode,
}

// Flow edges (session→turn, turn→turn): thick, solid indigo
const FLOW_EDGE_STYLE = {
  stroke: '#6366f1',
  strokeWidth: 2.5,
}
const FLOW_MARKER = {
  type: MarkerType.ArrowClosed,
  width: 16,
  height: 16,
  color: '#6366f1',
}

// Detail edges (turn→item): thin, dashed zinc
const DETAIL_EDGE_STYLE = {
  stroke: '#52525b',
  strokeWidth: 1.5,
  strokeDasharray: '5 4',
}
const DETAIL_MARKER = {
  type: MarkerType.ArrowClosed,
  width: 12,
  height: 12,
  color: '#52525b',
}

export function GraphCanvas() {
  const rawNodes = useGraphStore(s => s.nodes)
  const rawEdges = useGraphStore(s => s.edges)
  const selectedId = useGraphStore(s => s.selectedNodeId)
  const selectNode = useGraphStore(s => s.selectNode)
  const toggleTurn = useGraphStore(s => s.toggleTurn)
  const searchQuery = useGraphStore(s => s.searchQuery)

  const nodes = rawNodes.map(n => ({
    ...n,
    selected: n.id === selectedId,
    style: searchQuery
      ? n.data.label.toLowerCase().includes(searchQuery.toLowerCase())
        ? { filter: 'drop-shadow(0 0 8px rgba(99,102,241,0.7))' }
        : { opacity: 0.12 }
      : {},
  }))

  const edges = rawEdges.map(e => {
    const kind = (e.data as { kind?: EdgeKind })?.kind ?? 'detail'
    const isFlow = kind === 'flow'
    return {
      ...e,
      type: isFlow ? 'smoothstep' : 'default',
      style: isFlow ? FLOW_EDGE_STYLE : DETAIL_EDGE_STYLE,
      markerEnd: isFlow ? FLOW_MARKER : DETAIL_MARKER,
      animated: isFlow,
    }
  })

  const onNodeClick: NodeMouseHandler = useCallback(
    (_evt, node) => {
      const graphNode = node as Node<GraphNodeData>
      // Turn nodes: toggle expand/collapse
      if (graphNode.data.kind === 'turn') {
        toggleTurn(graphNode.data.turnId)
        return
      }
      selectNode(graphNode.id === selectedId ? null : graphNode.id)
    },
    [selectedId, selectNode, toggleTurn]
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
        minZoom={0.15}
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
            const kind = (n.data as GraphNodeData)?.kind
            if (kind === 'session') return '#6366f1'
            if (kind === 'turn') return '#818cf8'
            if (kind === 'error') return '#ef4444'
            if (kind === 'tool') return '#3b82f6'
            if (kind === 'patch') return '#22c55e'
            if (kind === 'plan') return '#a1a1aa'
            if (kind === 'summary') return '#d946ef'
            return '#eab308'
          }}
        />
      </ReactFlow>
    </div>
  )
}
