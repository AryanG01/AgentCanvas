import { create } from 'zustand'
import type { Edge } from '@xyflow/react'
import type { Node } from '@xyflow/react'
import { buildGraph } from '../lib/eventToGraph'
import { applyDagreLayout } from '../lib/layout'
import type { AppEvent, GraphNodeData, WsStatus } from '../lib/types'

interface GraphState {
  events: AppEvent[]
  nodes: Node<GraphNodeData>[]
  edges: Edge[]
  selectedNodeId: string | null
  searchQuery: string
  wsStatus: WsStatus

  addEvent: (event: AppEvent) => void
  selectNode: (id: string | null) => void
  setSearch: (q: string) => void
  setWsStatus: (s: WsStatus) => void
}

export const useGraphStore = create<GraphState>((set, get) => ({
  events: [],
  nodes: [],
  edges: [],
  selectedNodeId: null,
  searchQuery: '',
  wsStatus: 'disconnected',

  addEvent: (event) => {
    const events = [...get().events, event]
    const { nodes, edges } = buildGraph(events)
    const laidOut = applyDagreLayout(nodes, edges)
    set({ events, nodes: laidOut, edges })
  },

  selectNode: (id) => set({ selectedNodeId: id }),
  setSearch: (q) => set({ searchQuery: q }),
  setWsStatus: (s) => set({ wsStatus: s }),
}))
