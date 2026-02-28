import { useEffect, useMemo, useState } from 'react'
import { useGraphStore } from '../store/graphStore'
import type { SessionInfo } from '../lib/types'

interface SessionGroup {
  cwd: string
  project: string
  sessions: SessionInfo[]
  latestMs: number
}

function projectFromCwd(cwd: string): string {
  const parts = cwd.split('/').filter(Boolean)
  return parts.length > 0 ? parts[parts.length - 1] : 'unknown'
}

function parseSessionMs(session: SessionInfo): number {
  const ms = Date.parse(`${session.date}T${session.time}:00Z`)
  return Number.isNaN(ms) ? 0 : ms
}

function formatRelativeAge(session: SessionInfo): string {
  const createdAtMs = parseSessionMs(session)
  if (!createdAtMs) return session.date

  const diffMs = Date.now() - createdAtMs
  const minute = 60_000
  const hour = 60 * minute
  const day = 24 * hour
  const week = 7 * day
  const month = 30 * day

  if (diffMs < hour) return `${Math.max(1, Math.floor(diffMs / minute))}m`
  if (diffMs < day) return `${Math.floor(diffMs / hour)}h`
  if (diffMs < week) return `${Math.floor(diffMs / day)}d`
  if (diffMs < month) return `${Math.floor(diffMs / week)}w`
  return `${Math.floor(diffMs / month)}mo`
}

function sessionTitle(session: SessionInfo): string {
  const userTitle = session.title?.trim()
  if (userTitle) return userTitle

  const cleanId = session.id.trim()
  if (cleanId && !cleanId.startsWith('thread-')) return cleanId

  return `${session.turns} turn${session.turns === 1 ? '' : 's'} in ${session.source}`
}

export function SessionPicker() {
  const sessions = useGraphStore(s => s.sessions)
  const selectedSessionFile = useGraphStore(s => s.selectedSessionFile)
  const fetchSessions = useGraphStore(s => s.fetchSessions)
  const selectSession = useGraphStore(s => s.selectSession)

  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(new Set())
  const [copiedFile, setCopiedFile] = useState<string | null>(null)

  useEffect(() => {
    fetchSessions()
  }, [fetchSessions])

  const groups = useMemo<SessionGroup[]>(() => {
    const grouped = new Map<string, SessionGroup>()

    for (const session of sessions) {
      const cwd = session.cwd || 'unknown'
      if (!grouped.has(cwd)) {
        grouped.set(cwd, { cwd, project: projectFromCwd(cwd), sessions: [], latestMs: 0 })
      }
      grouped.get(cwd)!.sessions.push(session)
    }

    const sortedGroups = [...grouped.values()]
      .map(group => {
        const sortedSessions = [...group.sessions].sort((a, b) => parseSessionMs(b) - parseSessionMs(a))
        return {
          ...group,
          sessions: sortedSessions,
          latestMs: sortedSessions.length > 0 ? parseSessionMs(sortedSessions[0]) : 0,
        }
      })
      .sort((a, b) => b.latestMs - a.latestMs || a.project.localeCompare(b.project))

    return sortedGroups
  }, [sessions])

  const toggleGroup = (cwd: string) => {
    setCollapsedGroups(prev => {
      const next = new Set(prev)
      if (next.has(cwd)) {
        next.delete(cwd)
      } else {
        next.add(cwd)
      }
      return next
    })
  }

  const copyFilePath = async (file: string) => {
    if (!navigator.clipboard?.writeText) return

    try {
      await navigator.clipboard.writeText(file)
      setCopiedFile(file)
      window.setTimeout(() => {
        setCopiedFile(current => (current === file ? null : current))
      }, 1200)
    } catch {
      // clipboard access unavailable
    }
  }

  return (
    <aside className="h-[42vh] md:h-full w-full md:w-[360px] md:min-w-[320px] md:max-w-[380px] shrink-0 border-b md:border-b-0 md:border-r border-zinc-800/90 bg-[radial-gradient(circle_at_12%_4%,rgba(63,63,70,0.5),transparent_42%),linear-gradient(160deg,#18181b_0%,#0a0a0b_72%)] flex flex-col">
      <div className="px-5 pt-5 pb-3 border-b border-zinc-800/80">
        <div className="flex items-center justify-between">
          <h2 className="text-2xl leading-none font-semibold text-zinc-400 tracking-tight">Threads</h2>
          <div className="flex items-center gap-3 text-zinc-600">
            <button className="hover:text-zinc-300 transition-colors" aria-label="New folder">
              <svg className="w-[19px] h-[19px]" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.8} d="M4 7a2 2 0 012-2h4l2 2h6a2 2 0 012 2v1M5 10h14v7a2 2 0 01-2 2H7a2 2 0 01-2-2v-7zm7-4v4m-2-2h4" />
              </svg>
            </button>
            <button className="hover:text-zinc-300 transition-colors" aria-label="Sort threads">
              <svg className="w-[18px] h-[18px]" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.8} d="M4 6h16M7 12h10m-7 6h4" />
              </svg>
            </button>
          </div>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto px-2 py-3">
        {groups.length === 0 && (
          <div className="px-4 py-5 text-zinc-500 text-sm">
            No replay sessions found.
          </div>
        )}

        {groups.map(group => {
          const containsSelected = group.sessions.some(session => session.file === selectedSessionFile)
          const isCollapsed = collapsedGroups.has(group.cwd) && !containsSelected

          return (
            <section key={group.cwd} className="mb-3">
              <button
                onClick={() => toggleGroup(group.cwd)}
                className="w-full flex items-center gap-2 px-2.5 py-1.5 text-zinc-400 hover:text-zinc-200 transition-colors"
              >
                <svg className={`w-3.5 h-3.5 shrink-0 transition-transform ${isCollapsed ? '-rotate-90' : ''}`} fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
                </svg>
                <svg className="w-[17px] h-[17px] flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.8} d="M3 7a2 2 0 012-2h5l2 2h7a2 2 0 012 2v1H3V7zm0 3h18v7a2 2 0 01-2 2H5a2 2 0 01-2-2v-7z" />
                </svg>
                <span className="text-2xl leading-none font-medium tracking-tight truncate text-zinc-300">
                  {group.project}
                </span>
                <span className="ml-auto text-[11px] text-zinc-600 pr-1">
                  {group.sessions.length}
                </span>
              </button>

              {!isCollapsed && (
                <div className="mt-0.5 space-y-0.5 pl-5 pr-2">
                  {group.sessions.map(session => {
                    const isSelected = session.file === selectedSessionFile
                    return (
                      <button
                        key={session.file}
                        onClick={() => selectSession(session.file)}
                        className={`group w-full text-left rounded-xl px-3 py-2 transition-colors ${
                          isSelected
                            ? 'bg-zinc-700/45 ring-1 ring-zinc-500/45'
                            : 'hover:bg-zinc-800/75'
                        }`}
                      >
                        <div className="flex items-start gap-2">
                          <span className={`mt-1.5 h-2 w-2 rounded-full flex-shrink-0 ${
                            isSelected ? 'bg-zinc-200' : 'bg-zinc-600 group-hover:bg-zinc-400'
                          }`} />
                          <div className="min-w-0 flex-1">
                            <div className="flex items-start justify-between gap-2">
                              <p className={`text-[17px] leading-5 truncate ${
                                isSelected ? 'text-zinc-100' : 'text-zinc-200'
                              }`}>
                                {sessionTitle(session)}
                              </p>
                              <div className="flex items-center gap-1.5 pl-1">
                                <span className="text-sm leading-none text-zinc-500 flex-shrink-0 pt-1">
                                  {formatRelativeAge(session)}
                                </span>
                                <div className={`flex items-center gap-1 transition-opacity ${isSelected ? 'opacity-100' : 'opacity-0 group-hover:opacity-100'}`}>
                                  <button
                                    onClick={(event) => {
                                      event.stopPropagation()
                                      selectSession(session.file)
                                    }}
                                    className="h-6 w-6 rounded-md border border-zinc-700 bg-zinc-900/90 text-zinc-400 hover:text-zinc-100 hover:border-zinc-500 flex items-center justify-center transition-colors"
                                    aria-label="Open session"
                                    title="Open session"
                                  >
                                    <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.9} d="M9 5l7 7-7 7" />
                                    </svg>
                                  </button>
                                  <button
                                    onClick={(event) => {
                                      event.stopPropagation()
                                      void copyFilePath(session.file)
                                    }}
                                    className="h-6 w-6 rounded-md border border-zinc-700 bg-zinc-900/90 text-zinc-400 hover:text-zinc-100 hover:border-zinc-500 flex items-center justify-center transition-colors"
                                    aria-label="Copy session path"
                                    title={copiedFile === session.file ? 'Copied' : 'Copy session path'}
                                  >
                                    {copiedFile === session.file ? (
                                      <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                                      </svg>
                                    ) : (
                                      <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.8} d="M8 7a2 2 0 012-2h7a2 2 0 012 2v9a2 2 0 01-2 2h-7a2 2 0 01-2-2V7z" />
                                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.8} d="M5 17V9a2 2 0 012-2" />
                                      </svg>
                                    )}
                                  </button>
                                </div>
                              </div>
                            </div>
                            <p className="text-xs text-zinc-500 truncate mt-0.5">
                              {session.turns} turn{session.turns === 1 ? '' : 's'} · {session.source}
                            </p>
                          </div>
                        </div>
                      </button>
                    )
                  })}
                </div>
              )}
            </section>
          )
        })}
      </div>

      <div className="px-4 py-2 border-t border-zinc-800/80 text-xs text-zinc-600">
        {sessions.length} session{sessions.length === 1 ? '' : 's'}
      </div>
    </aside>
  )
}
