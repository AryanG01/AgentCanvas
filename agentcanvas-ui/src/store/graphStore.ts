import { create } from 'zustand'
import type { Edge, Node } from '@xyflow/react'
import { buildGraph } from '../lib/eventToGraph'
import { applyConversationLayout } from '../lib/layout'
import type { AppEvent, GraphNodeData, SessionInfo, WsStatus } from '../lib/types'

interface GraphState {
  events: AppEvent[]
  nodes: Node<GraphNodeData>[]
  edges: Edge[]
  turnOrder: string[]
  selectedNodeId: string | null
  searchQuery: string
  wsStatus: WsStatus
  wsUrl: string
  jsonlWsUrl: string
  sessions: SessionInfo[]
  selectedSessionFile: string | null
  expandedTurns: Set<string>

  addEvent: (event: AppEvent) => void
  updateTurnLabel: (turnId: string, label: string) => void
  selectNode: (id: string | null) => void
  setSearch: (q: string) => void
  setWsStatus: (s: WsStatus) => void
  setWsUrl: (url: string) => void
  setJsonlWsUrl: (url: string) => void
  fetchSessions: () => Promise<void>
  selectSession: (file: string) => void
  toggleTurn: (turnId: string) => void
  reset: () => void
}

const REPLAY_API = import.meta.env.VITE_REPLAY_API ?? '/api/sessions'
const BASE_WS_URL = import.meta.env.VITE_WS_URL ?? 'ws://localhost:5173/ws'
const BASE_JSONL_WS_URL = import.meta.env.VITE_JSONL_WS_URL ?? 'ws://localhost:3737'

const initialState = {
  events: [] as AppEvent[],
  nodes: [] as Node<GraphNodeData>[],
  edges: [] as Edge[],
  turnOrder: [] as string[],
  selectedNodeId: null as string | null,
  searchQuery: '',
  wsStatus: 'disconnected' as WsStatus,
  wsUrl: '',  // empty = don't connect until a session is selected
  jsonlWsUrl: BASE_JSONL_WS_URL,  // JSONL WebSocket URL for codex-tui
  sessions: [] as SessionInfo[],
  selectedSessionFile: null as string | null,
  expandedTurns: new Set<string>(),
}

function recompute(events: AppEvent[], expandedTurns: Set<string>) {
  const { nodes: allNodes, edges: allEdges, turnOrder } = buildGraph(events)

  // Filter: only show event nodes for expanded turns
  const nodes = allNodes.filter(n => {
    const kind = n.data.kind
    if (kind === 'session' || kind === 'turn') return true
    // Event nodes: only show if their parent turn is expanded
    return expandedTurns.has(n.data.turnId)
  })

  const visibleIds = new Set(nodes.map(n => n.id))
  const edges = allEdges.filter(e => visibleIds.has(e.source) && visibleIds.has(e.target))

  // Annotate turn nodes with item count and collapsed state
  for (const node of nodes) {
    if (node.data.kind === 'turn') {
      const itemCount = allNodes.filter(n => n.data.turnId === node.data.turnId && n.data.kind !== 'turn').length
      node.data = { ...node.data, collapsed: !expandedTurns.has(node.data.turnId), itemCount }
    }
  }

  const laidOut = applyConversationLayout(nodes, edges)
  return { events, nodes: laidOut, edges, turnOrder }
}

export const useGraphStore = create<GraphState>((set, get) => ({
  ...initialState,

  addEvent: (event) => {
    const events = [...get().events, event]
    set(recompute(events, get().expandedTurns))
  },

  // Retroactively patch the turn label when the userMessage item arrives
  updateTurnLabel: (turnId, label) => {
    const events = get().events.map(e =>
      e.type === 'TurnStarted' && e.turnId === turnId
        ? { ...e, userPrompt: label }
        : e
    )
    set(recompute(events, get().expandedTurns))
  },

  selectNode: (id) => set({ selectedNodeId: id }),
  setSearch: (q) => set({ searchQuery: q }),
  setWsStatus: (s) => set({ wsStatus: s }),
  setWsUrl: (url) => set({ wsUrl: url }),
  setJsonlWsUrl: (url) => set({ jsonlWsUrl: url }),

  fetchSessions: async () => {
    try {
      const res = await fetch(REPLAY_API)
      if (!res.ok) return
      const sessions = await res.json() as SessionInfo[]
      set({ sessions })
    } catch {
      // replay server not running — ignore
    }
  },

  selectSession: (file) => {
    // Reset graph state and set new WS URL with ?file= param
    const wsBase = BASE_WS_URL.replace(/\?.*$/, '')  // strip existing params
    const wsUrl = `${wsBase}?file=${encodeURIComponent(file)}`
    set({
      ...initialState,
      sessions: get().sessions,  // preserve session list
      selectedSessionFile: file,
      wsUrl,
    })
  },

  toggleTurn: (turnId) => {
    const expanded = new Set(get().expandedTurns)
    if (expanded.has(turnId)) {
      expanded.delete(turnId)
    } else {
      expanded.add(turnId)
    }
    set({ expandedTurns: expanded, ...recompute(get().events, expanded) })
  },

  reset: () => set({ ...initialState }),
}))
