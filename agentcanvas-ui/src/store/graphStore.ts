import { create } from 'zustand'
import type { Edge, Node } from '@xyflow/react'
import { buildGraph } from '../lib/eventToGraph'
import { applyConversationLayout } from '../lib/layout'
import type { AppEvent, GraphNodeData, WsStatus } from '../lib/types'

interface GraphState {
  events: AppEvent[]
  nodes: Node<GraphNodeData>[]
  edges: Edge[]
  turnOrder: string[]
  selectedNodeId: string | null
  searchQuery: string
  wsStatus: WsStatus
  wsUrl: string

  addEvent: (event: AppEvent) => void
  updateTurnLabel: (turnId: string, label: string) => void
  selectNode: (id: string | null) => void
  setSearch: (q: string) => void
  setWsStatus: (s: WsStatus) => void
  setWsUrl: (url: string) => void
  reset: () => void
}

const initialState = {
  events: [] as AppEvent[],
  nodes: [] as Node<GraphNodeData>[],
  edges: [] as Edge[],
  turnOrder: [] as string[],
  selectedNodeId: null as string | null,
  searchQuery: '',
  wsStatus: 'disconnected' as WsStatus,
  wsUrl: import.meta.env.VITE_WS_URL ?? 'ws://localhost:5173/ws',
}

function recompute(events: AppEvent[]) {
  const { nodes, edges, turnOrder } = buildGraph(events)
  const laidOut = applyConversationLayout(nodes, edges)
  return { events, nodes: laidOut, edges, turnOrder }
}

export const useGraphStore = create<GraphState>((set, get) => ({
  ...initialState,

  addEvent: (event) => {
    const events = [...get().events, event]
    set(recompute(events))
  },

  // Retroactively patch the turn label when the userMessage item arrives
  updateTurnLabel: (turnId, label) => {
    const events = get().events.map(e =>
      e.type === 'TurnStarted' && e.turnId === turnId
        ? { ...e, userPrompt: label }
        : e
    )
    set(recompute(events))
  },

  selectNode: (id) => set({ selectedNodeId: id }),
  setSearch: (q) => set({ searchQuery: q }),
  setWsStatus: (s) => set({ wsStatus: s }),
  setWsUrl: (url) => set({ wsUrl: url }),
  reset: () => set({ ...initialState }),
}))
