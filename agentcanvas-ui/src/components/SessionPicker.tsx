import { useEffect, useState } from 'react'
import { useGraphStore } from '../store/graphStore'

export function SessionPicker() {
  const sessions = useGraphStore(s => s.sessions)
  const selectedSessionFile = useGraphStore(s => s.selectedSessionFile)
  const fetchSessions = useGraphStore(s => s.fetchSessions)
  const selectSession = useGraphStore(s => s.selectSession)
  const [open, setOpen] = useState(false)

  useEffect(() => {
    if (!open) return
    fetchSessions()
  }, [fetchSessions, open])

  const selected = sessions.find(s => s.file === selectedSessionFile)
  const buttonLabel = selected
    ? `${selected.date} ${selected.time} — ${selected.turns} turn${selected.turns !== 1 ? 's' : ''}`
    : sessions.length > 0
      ? `${sessions.length} replay session${sessions.length !== 1 ? 's' : ''}`
      : 'Replay sessions'

  return (
    <div className="absolute top-4 left-4 z-20">
      <div className="relative">
        <button
          onClick={() => setOpen(!open)}
          className="flex items-center gap-2 bg-zinc-900/95 backdrop-blur-sm border border-zinc-700 rounded-xl px-3 py-2 shadow-2xl shadow-black/50 hover:border-indigo-500 transition-colors"
        >
          <svg className="w-4 h-4 text-indigo-400 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 11H5m14 0a2 2 0 012 2v6a2 2 0 01-2 2H5a2 2 0 01-2-2v-6a2 2 0 012-2m14 0V9a2 2 0 00-2-2M5 11V9a2 2 0 012-2m0 0V5a2 2 0 012-2h6a2 2 0 012 2v2M7 7h10" />
          </svg>
          <span className="text-sm text-zinc-100 font-medium">
            {buttonLabel}
          </span>
          <svg className={`w-3 h-3 text-zinc-500 transition-transform ${open ? 'rotate-180' : ''}`} fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
          </svg>
        </button>

        {open && (
          <div className="absolute top-full left-0 mt-1 w-96 max-h-80 overflow-y-auto bg-zinc-900/98 backdrop-blur-sm border border-zinc-700 rounded-xl shadow-2xl shadow-black/60">
            {sessions.length === 0 ? (
              <div className="px-3 py-3 text-xs text-zinc-500">
                No replay sessions found on the replay endpoint.
              </div>
            ) : (
              sessions.map((session) => {
                const isSelected = session.file === selectedSessionFile
                return (
                  <button
                    key={session.file}
                    onClick={() => {
                      selectSession(session.file)
                      setOpen(false)
                    }}
                    className={`w-full text-left px-3 py-2.5 hover:bg-zinc-800 transition-colors border-b border-zinc-800 last:border-b-0 ${
                      isSelected ? 'bg-indigo-950/50 border-indigo-800' : ''
                    }`}
                  >
                    <div className="flex items-center justify-between">
                      <span className="text-sm text-zinc-100 font-medium">
                        {session.date} {session.time}
                      </span>
                      <div className="flex items-center gap-2">
                        <span className={`text-[10px] font-semibold px-1.5 py-0.5 rounded ${
                          session.source === 'cli'
                            ? 'bg-green-900/50 text-green-400'
                            : 'bg-blue-900/50 text-blue-400'
                        }`}>
                          {session.source}
                        </span>
                        <span className="text-[10px] text-zinc-500">
                          {session.turns} turn{session.turns !== 1 ? 's' : ''}
                        </span>
                      </div>
                    </div>
                    <p className="text-[11px] text-zinc-500 mt-0.5 truncate">{session.cwd}</p>
                  </button>
                )
              })
            )}
          </div>
        )}
      </div>
    </div>
  )
}
