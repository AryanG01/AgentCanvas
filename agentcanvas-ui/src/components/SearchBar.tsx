import { useGraphStore } from '../store/graphStore'

const statusConfig = {
  connected:    { dot: 'bg-green-400',                   text: 'text-green-400',  label: 'Live' },
  connecting:   { dot: 'bg-yellow-400 animate-pulse',    text: 'text-yellow-400', label: 'Connecting…' },
  disconnected: { dot: 'bg-zinc-600',                    text: 'text-zinc-500',   label: 'Mock' },
}

export function SearchBar() {
  const query = useGraphStore(s => s.searchQuery)
  const setSearch = useGraphStore(s => s.setSearch)
  const wsStatus = useGraphStore(s => s.wsStatus)
  const nodes = useGraphStore(s => s.nodes)
  const cfg = statusConfig[wsStatus]

  const turnCount = nodes.filter(n => n.data.kind === 'turn').length
  const eventCount = nodes.filter(n => n.data.kind !== 'turn').length
  const matchCount = query
    ? nodes.filter(n => n.data.label.toLowerCase().includes(query.toLowerCase())).length
    : null

  return (
    <div className="absolute top-4 left-1/2 -translate-x-1/2 z-10">
      <div className="flex items-center gap-2 bg-zinc-900/95 backdrop-blur-sm border border-zinc-700 rounded-xl px-3 py-2 shadow-2xl shadow-black/50">
        {/* Status indicator */}
        <div className="flex items-center gap-1.5 pr-2 border-r border-zinc-700">
          <span className={`h-2 w-2 rounded-full flex-shrink-0 ${cfg.dot}`} />
          <span className={`text-[10px] font-semibold ${cfg.text}`}>{cfg.label}</span>
        </div>

        {/* Search input */}
        <div className="flex items-center gap-2">
          <svg className="w-3.5 h-3.5 text-zinc-500 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
          </svg>
          <input
            type="text"
            placeholder="Search nodes…"
            value={query}
            onChange={e => setSearch(e.target.value)}
            className="bg-transparent text-sm text-zinc-100 placeholder-zinc-600 outline-none w-44"
          />
          {query ? (
            <button
              onClick={() => setSearch('')}
              className="text-zinc-500 hover:text-white text-xs leading-none transition-colors"
            >
              ✕
            </button>
          ) : null}
        </div>

        {/* Stats */}
        <div className="flex items-center gap-2 pl-2 border-l border-zinc-700">
          {matchCount !== null ? (
            <span className="text-[10px] text-indigo-400 font-semibold">{matchCount} match{matchCount !== 1 ? 'es' : ''}</span>
          ) : (
            <>
              <span className="text-[10px] text-zinc-500">{turnCount} turn{turnCount !== 1 ? 's' : ''}</span>
              <span className="text-zinc-700">·</span>
              <span className="text-[10px] text-zinc-500">{eventCount} event{eventCount !== 1 ? 's' : ''}</span>
            </>
          )}
        </div>
      </div>
    </div>
  )
}
