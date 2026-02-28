import { useState } from 'react'
import { useGraphStore } from '../store/graphStore'

export function ConnectPanel() {
  const wsStatus = useGraphStore(s => s.wsStatus)
  const wsUrl = useGraphStore(s => s.wsUrl)
  const setWsUrl = useGraphStore(s => s.setWsUrl)
  const reset = useGraphStore(s => s.reset)
  const [editUrl, setEditUrl] = useState(wsUrl)
  const [open, setOpen] = useState(false)

  if (wsStatus === 'connected') return null  // hide when connected

  return (
    <div className="absolute bottom-4 left-1/2 -translate-x-1/2 z-20">
      {!open ? (
        <button
          onClick={() => setOpen(true)}
          className="flex items-center gap-2 bg-indigo-600 hover:bg-indigo-500 text-white text-sm font-medium px-4 py-2 rounded-xl shadow-xl transition-colors"
        >
          <span className="h-2 w-2 rounded-full bg-red-400" />
          Connect to Codex session
        </button>
      ) : (
        <div className="bg-zinc-900 border border-zinc-700 rounded-xl p-4 shadow-2xl w-[380px] space-y-3">
          <div className="flex items-center justify-between">
            <span className="text-sm font-semibold text-zinc-100">Connect to app-server</span>
            <button onClick={() => setOpen(false)} className="text-zinc-500 hover:text-white text-lg">×</button>
          </div>
          <p className="text-xs text-zinc-400 leading-relaxed">
            Start the Codex app-server with WebSocket transport, then paste the URL below.
          </p>
          <div className="bg-zinc-800 rounded-lg p-2.5 font-mono text-xs text-green-300 border border-zinc-700">
            codex app-server --listen ws://127.0.0.1:8080
          </div>
          <div className="flex gap-2">
            <input
              type="text"
              value={editUrl}
              onChange={e => setEditUrl(e.target.value)}
              className="flex-1 bg-zinc-800 border border-zinc-700 rounded-lg px-3 py-1.5 text-sm text-zinc-100 outline-none focus:border-indigo-500"
              placeholder="ws://localhost:8080"
            />
            <button
              onClick={() => {
                reset()
                setWsUrl(editUrl)
                setOpen(false)
              }}
              className="bg-indigo-600 hover:bg-indigo-500 text-white text-sm font-medium px-3 py-1.5 rounded-lg transition-colors"
            >
              Connect
            </button>
          </div>
        </div>
      )}
    </div>
  )
}
