import { useEffect } from 'react'
import { ReactFlowProvider } from '@xyflow/react'
import { GraphCanvas } from './components/GraphCanvas'
import { EvidencePanel } from './components/EvidencePanel'
import { SearchBar } from './components/SearchBar'
import { useAppServerWS } from './hooks/useAppServerWS'
import { useGraphStore } from './store/graphStore'
import { MOCK_EVENTS } from './lib/mockEvents'

const USE_MOCK = import.meta.env.VITE_USE_MOCK === 'true'

function AppInner() {
  const addEvent = useGraphStore(s => s.addEvent)
  useAppServerWS()

  // Load mock events with staggered delay to simulate streaming
  useEffect(() => {
    if (!USE_MOCK) return
    MOCK_EVENTS.forEach((event, i) => {
      setTimeout(() => addEvent(event), i * 300)
    })
  }, [])

  return (
    <div className="w-screen h-screen relative overflow-hidden">
      <SearchBar />
      <GraphCanvas />
      <EvidencePanel />
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
