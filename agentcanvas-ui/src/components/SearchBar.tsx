import { useGraphStore } from '../store/graphStore'

export function SearchBar() {
  const query = useGraphStore(s => s.searchQuery)
  const setSearch = useGraphStore(s => s.setSearch)
  const wsStatus = useGraphStore(s => s.wsStatus)

  const statusColor = {
    connected: 'bg-green-400',
    connecting: 'bg-yellow-400 animate-pulse',
    disconnected: 'bg-red-500',
  }[wsStatus]

  return (
    <div className="absolute top-3 left-1/2 -translate-x-1/2 z-10 flex items-center gap-3">
      <div className="flex items-center gap-2 bg-zinc-900 border border-zinc-700 rounded-lg px-3 py-1.5 shadow-lg">
        <span className={`h-2 w-2 rounded-full flex-shrink-0 ${statusColor}`} title={wsStatus} />
        <input
          type="text"
          placeholder="Search nodes..."
          value={query}
          onChange={e => setSearch(e.target.value)}
          className="bg-transparent text-sm text-zinc-100 placeholder-zinc-500 outline-none w-52"
        />
        {query && (
          <button onClick={() => setSearch('')} className="text-zinc-400 hover:text-white">
            ×
          </button>
        )}
      </div>
    </div>
  )
}
