import { useEffect } from 'react'
import { ReactFlowProvider } from '@xyflow/react'
import { GraphCanvas } from './components/GraphCanvas'
import { EvidencePanel } from './components/EvidencePanel'
import { SearchBar } from './components/SearchBar'
import { SessionPicker } from './components/SessionPicker'
import { ConnectPanel } from './components/ConnectPanel'
import { useAppServerWS } from './hooks/useAppServerWS'
import { useCodexJsonlWS } from './hooks/useCodexJsonlWS'
import { useGraphStore } from './store/graphStore'
import { MOCK_EVENTS } from './lib/mockEvents'

const USE_MOCK = import.meta.env.VITE_USE_MOCK === 'true'
const USE_JSONL = import.meta.env.VITE_USE_JSONL !== 'false'

function AppInner() {
  const addEvent = useGraphStore(s => s.addEvent)
  const nodes = useGraphStore(s => s.nodes)
  const wsStatus = useGraphStore(s => s.wsStatus)

  // Use JSONL hook or app-server hook based on env var
  // Always call both hooks unconditionally (React Rules of Hooks), but pass null to disable the inactive one
  const jsonlUrl = USE_JSONL ? undefined : null  // undefined = use store default, null = disable
  const appServerUrl = USE_JSONL ? null : undefined  // null = disable, undefined = use store default
  console.log('[App] USE_JSONL:', USE_JSONL)
  console.log('[App] jsonlUrl:', jsonlUrl)

  useCodexJsonlWS(jsonlUrl)   // Always called, but null disables it
  useAppServerWS(appServerUrl) // Always called, but null disables it

  useEffect(() => {
    if (!USE_MOCK) return
    const timers = MOCK_EVENTS.map((event, i) =>
      setTimeout(() => addEvent(event), i * 300)
    )
    return () => timers.forEach(clearTimeout)
  }, [])

  const isEmpty = nodes.length === 0

  return (
    <div className="w-screen h-screen relative overflow-hidden bg-zinc-950">
      <SessionPicker />
      <SearchBar />
      <GraphCanvas />
      <EvidencePanel />
      {!USE_MOCK && !USE_JSONL && <ConnectPanel />}

      {/* JSONL connection status indicator */}
      {USE_JSONL && (
        <div className="absolute top-4 right-4 flex items-center gap-2 text-xs">
          <div className={`w-2 h-2 rounded-full ${
            wsStatus === 'connected' ? 'bg-green-500' :
            wsStatus === 'connecting' ? 'bg-yellow-500 animate-pulse' :
            'bg-red-500'
          }`} />
          <span className="text-zinc-400">
            {wsStatus === 'connected' ? 'Connected' :
             wsStatus === 'connecting' ? 'Connecting...' :
             'Disconnected'}
          </span>
        </div>
      )}

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
