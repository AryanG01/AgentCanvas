import { useEffect } from 'react'
import { ReactFlowProvider } from '@xyflow/react'
import { GraphCanvas } from './components/GraphCanvas'
import { EvidencePanel } from './components/EvidencePanel'
import { SearchBar } from './components/SearchBar'
import { SessionPicker } from './components/SessionPicker'
import { SessionTabs } from './components/SessionTabs'
import { ConnectPanel } from './components/ConnectPanel'
import { useAppServerWS } from './hooks/useAppServerWS'
import { useGraphStore } from './store/graphStore'
import { MOCK_EVENTS } from './lib/mockEvents'

const USE_MOCK = import.meta.env.VITE_USE_MOCK === 'true'

function AppInner() {
  const addEvent = useGraphStore(s => s.addEvent)
  const nodes = useGraphStore(s => s.nodes)
  const setWsUrl = useGraphStore(s => s.setWsUrl)
  useAppServerWS()

  useEffect(() => {
    if (!USE_MOCK) return
    const timers = MOCK_EVENTS.map((event, i) =>
      setTimeout(() => addEvent(event), i * 300)
    )
    return () => timers.forEach(clearTimeout)
  }, [])

  useEffect(() => {
    const params = new URLSearchParams(window.location.search)
    if (!params.has('autoconnect') && !params.has('watch')) return

    const baseUrl = import.meta.env.VITE_WS_URL ?? '/ws'
    const file = params.get('file')
    const nextUrl = file
      ? `${baseUrl}?file=${encodeURIComponent(file)}`
      : `${baseUrl}?watch=1`
    setWsUrl(nextUrl)
  }, [setWsUrl])

  const isEmpty = nodes.length === 0

  return (
    <div className="w-screen h-screen relative overflow-hidden bg-zinc-950">
      <SessionPicker />
      <SessionTabs />
      <SearchBar />
      <GraphCanvas />
      <EvidencePanel />
      {!USE_MOCK && <ConnectPanel />}

      {/* Empty state */}
      {isEmpty && (
        <div className="absolute inset-0 flex flex-col items-center justify-center pointer-events-none">
          <div className="text-center space-y-2">
            <p className="text-zinc-600 text-sm font-medium">No session data yet</p>
            <p className="text-zinc-700 text-xs">
              {USE_MOCK ? 'Loading mock events…' : 'Connect to a Codex session to begin'}
            </p>
          </div>
        </div>
      )}
    </div>
  )
}

export default function App() {
  return (
    <ReactFlowProvider>
      <AppInner />
    </ReactFlowProvider>
  )
}
