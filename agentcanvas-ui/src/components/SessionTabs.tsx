import { useGraphStore } from '../store/graphStore'

function shortSessionId(sessionId: string): string {
  if (sessionId.length <= 20) return sessionId
  return `…${sessionId.slice(-18)}`
}

export function SessionTabs() {
  const sessionIds = useGraphStore(s => s.sessionIds)
  const activeSessionId = useGraphStore(s => s.activeSessionId)
  const selectActiveSession = useGraphStore(s => s.selectActiveSession)

  if (sessionIds.length <= 1) return null

  return (
    <div className="absolute top-16 left-4 z-20">
      <div className="flex max-w-[70vw] items-center gap-2 overflow-x-auto rounded-xl border border-zinc-700 bg-zinc-900/95 px-2 py-2 shadow-2xl shadow-black/50 backdrop-blur-sm">
        {sessionIds.map((sessionId, index) => {
          const isActive = sessionId === activeSessionId
          return (
            <button
              key={sessionId}
              onClick={() => selectActiveSession(sessionId)}
              className={`flex items-center gap-2 rounded-lg border px-2.5 py-1.5 text-xs transition-colors ${
                isActive
                  ? 'border-indigo-500 bg-indigo-950/50 text-indigo-200'
                  : 'border-zinc-700 bg-zinc-900 text-zinc-400 hover:border-zinc-500 hover:text-zinc-200'
              }`}
            >
              <span className="text-[10px] font-semibold text-zinc-500">S{index + 1}</span>
              <span className="font-mono">{shortSessionId(sessionId)}</span>
              {isActive && <span className="h-1.5 w-1.5 rounded-full bg-green-400" />}
            </button>
          )
        })}
      </div>
    </div>
  )
}
