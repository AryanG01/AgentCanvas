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
  sessionIds: string[]
  activeSessionId: string | null
  turnToSession: Map<string, string>
  selectedNodeId: string | null
  searchQuery: string
  wsStatus: WsStatus
  wsUrl: string
  sessions: SessionInfo[]
  selectedSessionFile: string | null
  expandedTurns: Set<string>

  addEvent: (event: AppEvent) => void
  updateTurnLabel: (turnId: string, label: string) => void
  selectNode: (id: string | null) => void
  setSearch: (q: string) => void
  setWsStatus: (s: WsStatus) => void
  setWsUrl: (url: string) => void
  fetchSessions: () => Promise<void>
  selectSession: (file: string) => void
  selectActiveSession: (sessionId: string) => void
  toggleTurn: (turnId: string) => void
  reset: () => void
}

const REPLAY_API = import.meta.env.VITE_REPLAY_API ?? '/api/sessions'
const BASE_WS_URL = import.meta.env.VITE_WS_URL ?? 'ws://localhost:5173/ws'

function createInitialState() {
  return {
    events: [] as AppEvent[],
    nodes: [] as Node<GraphNodeData>[],
    edges: [] as Edge[],
    turnOrder: [] as string[],
    sessionIds: [] as string[],
    activeSessionId: null as string | null,
    turnToSession: new Map<string, string>(),
    selectedNodeId: null as string | null,
    searchQuery: '',
    wsStatus: 'disconnected' as WsStatus,
    wsUrl: '',  // empty = don't connect until a session is selected
    sessions: [] as SessionInfo[],
    selectedSessionFile: null as string | null,
    expandedTurns: new Set<string>(),
  }
}

function eventSessionId(
  event: AppEvent,
  turnToSession: Map<string, string>,
  fallbackSessionId: string | null
): string | null {
  if (event.type === 'ThreadStarted') return event.threadId
  if (event.type === 'TurnStarted') return event.sessionId
  return turnToSession.get(event.turnId) ?? fallbackSessionId
}

function recompute(
  events: AppEvent[],
  expandedTurns: Set<string>,
  activeSessionId: string | null,
  turnToSession: Map<string, string>
) {
  const scopedEvents = activeSessionId
    ? events.filter(event => eventSessionId(event, turnToSession, activeSessionId) === activeSessionId)
    : events
  const { nodes: allNodes, edges: allEdges, turnOrder } = buildGraph(scopedEvents)

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
  ...createInitialState(),

  addEvent: (event) => {
    const state = get()
    const events = [...state.events, event]
    const turnToSession = new Map(state.turnToSession)
    if (event.type === 'TurnStarted') turnToSession.set(event.turnId, event.sessionId)

    const newestSessionId = eventSessionId(event, turnToSession, state.activeSessionId)
    const activeSessionId = newestSessionId ?? state.activeSessionId
    const sessionIds = newestSessionId && !state.sessionIds.includes(newestSessionId)
      ? [...state.sessionIds, newestSessionId]
      : state.sessionIds
    const selectedNodeId =
      newestSessionId && state.activeSessionId && newestSessionId !== state.activeSessionId
        ? null
        : state.selectedNodeId

    set({
      turnToSession,
      sessionIds,
      activeSessionId,
      selectedNodeId,
      ...recompute(events, state.expandedTurns, activeSessionId, turnToSession),
    })
  },

  // Retroactively patch the turn label when the userMessage item arrives
  updateTurnLabel: (turnId, label) => {
    const events = get().events.map(e =>
      e.type === 'TurnStarted' && e.turnId === turnId
        ? { ...e, userPrompt: label }
        : e
    )
    set(recompute(events, get().expandedTurns, get().activeSessionId, get().turnToSession))
  },

  selectNode: (id) => set({ selectedNodeId: id }),
  setSearch: (q) => set({ searchQuery: q }),
  setWsStatus: (s) => set({ wsStatus: s }),
  setWsUrl: (url) => set({ wsUrl: url }),

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
    const next = createInitialState()
    set({
      ...next,
      sessions: get().sessions,  // preserve session list
      selectedSessionFile: file,
      wsUrl,
    })
  },

  selectActiveSession: (sessionId) => {
    const state = get()
    if (state.activeSessionId === sessionId) return
    set({
      activeSessionId: sessionId,
      selectedNodeId: null,
      ...recompute(state.events, state.expandedTurns, sessionId, state.turnToSession),
    })
  },

  toggleTurn: (turnId) => {
    const state = get()
    const expanded = new Set(state.expandedTurns)
    if (expanded.has(turnId)) {
      expanded.delete(turnId)
    } else {
      expanded.add(turnId)
    }
    set({
      expandedTurns: expanded,
      ...recompute(state.events, expanded, state.activeSessionId, state.turnToSession),
    })
  },

  reset: () => set({ ...createInitialState() }),
}))
