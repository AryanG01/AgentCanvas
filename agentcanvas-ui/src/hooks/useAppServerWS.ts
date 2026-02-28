import { useEffect, useRef } from 'react'
import { useGraphStore } from '../store/graphStore'
import type { AppEvent } from '../lib/types'

const MAX_BACKOFF = 30_000
const USE_MOCK = import.meta.env.VITE_USE_MOCK === 'true'
const REMOTE_SUMMARY_ENABLED = import.meta.env.VITE_SUMMARY_REMOTE === 'true'
const SUMMARY_API_URL = import.meta.env.VITE_SUMMARY_API_URL as string | undefined

type Recordish = Record<string, unknown>

function asRecord(value: unknown): Recordish | null {
  return value !== null && typeof value === 'object' ? value as Recordish : null
}

function firstString(values: ReadonlyArray<unknown>): string {
  for (const value of values) {
    if (typeof value === 'string' && value.length > 0) return value
  }
  return ''
}

function toNumber(value: unknown): number {
  const num = Number(value)
  return Number.isFinite(num) ? num : 0
}

function parseStringList(value: unknown): string[] {
  if (!Array.isArray(value)) return []
  return value.filter((entry): entry is string => typeof entry === 'string')
}

function parseSummaryCommand(entry: unknown): { command: string; exitCode: number | null } | null {
  if (!entry || typeof entry !== 'object') return null
  const obj = entry as Recordish
  const command = firstString([obj.command as string, obj.commandText as string])
  if (!command) return null

  const exitCode = obj.exitCode === null || obj.exit_code === null
    ? null
    : toNumber(obj.exitCode ?? obj.exit_code)

  return { command, exitCode }
}

function parseSummaryCommands(value: unknown): Array<{ command: string; exitCode: number | null }> {
  if (!Array.isArray(value)) {
    const asString = parseStringList(value)
    if (asString.length === 0) return []
    return asString.map(command => ({ command, exitCode: null }))
  }

  const result: Array<{ command: string; exitCode: number | null }> = []
  for (const entry of value) {
    const parsed = parseSummaryCommand(entry)
    if (parsed) result.push(parsed)
  }
  return result
}

function toSummaryStatus(value: unknown): string {
  if (typeof value === 'string' && value.length > 0) return value
  return 'unknown'
}

function normalizeSummaryEvent(params: Recordish): AppEvent | null {
  const node = asRecord(params.node)
    ?? asRecord(params.summaryNode)
    ?? asRecord(params.data)
    ?? params

  const turnId = firstString([
    params.turnId as string,
    params.turn_id as string,
    node.turnId as string,
    node.turn_id as string,
  ])
  if (!turnId) return null

  const brief = asRecord(node.brief) ?? asRecord(params.brief) ?? {}
  const counts = asRecord(node.counts) ?? asRecord(params.counts) ?? {}
  const digest = asRecord(node.digest) ?? asRecord(params.digest) ?? {}
  const lineage = asRecord(node.lineage) ?? asRecord(params.lineage) ?? {}
  const evidence = asRecord(node.evidence) ?? asRecord(params.evidence) ?? {}

  const summaryText = firstString([
    node.summaryText as string,
    node.summary as string,
    params.summary as string,
    params.summaryText as string,
  ])

  return {
    type: 'SummaryNode',
    id: firstString([
      node.node_id as string,
      node.nodeId as string,
      params.node_id as string,
      params.nodeId as string,
      `summary-${turnId}`,
    ]),
    turnId,
    nodeType: firstString([node.nodeType as string, node.node_type as string]) === 'phase' ? 'phase' : 'turn',
    status: toSummaryStatus(node.status ?? params.status),
    summaryText,
    brief: {
      signal: firstString([brief.signal as string, brief.signalType as string, node.signal as string, node.signalType as string]) || 'status_only',
      agentMessage: firstString([brief.agentMessage as string, brief.agent_message as string, node.agentMessage as string, node.agent_message as string]) || null,
      primaryCommand: firstString([brief.primaryCommand as string, brief.primary_command as string, node.primaryCommand as string, node.primary_command as string]) || null,
      primaryFilePath: firstString([brief.primaryFilePath as string, brief.primary_file_path as string, node.primaryFilePath as string, node.primary_file_path as string]) || null,
      primaryError: firstString([brief.primaryError as string, brief.primary_error as string, node.primaryError as string, node.primary_error as string]) || null,
    },
    counts: {
      commandsTotal: toNumber(counts.commandsTotal ?? counts.commands_total),
      commandsIndexed: toNumber(counts.commandsIndexed ?? counts.commands_indexed),
      commandsOmitted: toNumber(counts.commandsOmitted ?? counts.commands_omitted),
      filePathsTotal: toNumber(counts.filePathsTotal ?? counts.file_paths_total),
      filePathsIndexed: toNumber(counts.filePathsIndexed ?? counts.file_paths_indexed),
      filePathsOmitted: toNumber(counts.filePathsOmitted ?? counts.file_paths_omitted),
      errorsTotal: toNumber(counts.errorsTotal ?? counts.errors_total),
      errorsIndexed: toNumber(counts.errorsIndexed ?? counts.errors_indexed),
      errorsOmitted: toNumber(counts.errorsOmitted ?? counts.errors_omitted),
    },
    digest: {
      commandExamples: parseStringList(digest.commandExamples ?? digest.command_examples ?? digest.commands),
      filePathExamples: parseStringList(digest.filePathExamples ?? digest.file_path_examples ?? digest.filePaths ?? digest.file_paths),
      errorExamples: parseStringList(digest.errorExamples ?? digest.error_examples ?? digest.errors),
    },
    lineage: {
      parentTurnId: firstString([lineage.parentTurnId as string, lineage.parent_turn_id as string]) || null,
      childTurnId: firstString([lineage.childTurnId as string, lineage.child_turn_id as string]) || null,
      childTurnIds: parseStringList(lineage.childTurnIds ?? lineage.child_turn_ids),
      forkedFromThreadId: firstString([lineage.forkedFromThreadId as string, lineage.forked_from_thread_id as string]) || null,
      startedAfterRollback: Boolean(lineage.startedAfterRollback ?? lineage.started_after_rollback),
      wasRolledBack: Boolean(lineage.wasRolledBack ?? lineage.was_rolled_back),
    },
    evidence: {
      childTurnIds: parseStringList(evidence.childTurnIds ?? evidence.child_turn_ids),
      filePaths: parseStringList(evidence.filePaths ?? evidence.file_paths),
      commands: parseSummaryCommands(evidence.commands),
      errors: parseStringList(evidence.errors),
    },
    ts: Date.now(),
  }
}

type TurnStatus = 'success' | 'error' | 'cancelled'

interface LocalTurnSummaryState {
  sessionId: string | null
  parentTurnId: string | null
  childTurnIds: string[]
  assistantMessage: string | null
  commands: Array<{ command: string; exitCode: number | null }>
  filePaths: string[]
  errors: string[]
  finalStatus: TurnStatus | null
  remoteSummary: boolean
}

function pushUnique(values: string[], value: string, maxSize = 64) {
  const next = value.trim()
  if (!next) return
  if (values.includes(next)) return
  values.push(next)
  if (values.length > maxSize) values.shift()
}

function summarizeStatus(status: TurnStatus | null): string {
  if (status === 'success') return 'completed'
  if (status === 'error') return 'failed'
  if (status === 'cancelled') return 'cancelled'
  return 'in_progress'
}

function summarizeSignal(state: LocalTurnSummaryState): string {
  const hasFailedCommand = state.commands.some(cmd => cmd.exitCode !== null && cmd.exitCode !== 0)
  if (state.finalStatus === 'error' || state.errors.length > 0 || hasFailedCommand) return 'error'
  if (state.assistantMessage) return 'agent_message'
  if (state.commands.length > 0) return 'command_activity'
  if (state.filePaths.length > 0) return 'file_change'
  return 'status_only'
}

function summarizeText(state: LocalTurnSummaryState): string {
  const assistant = state.assistantMessage?.trim()
  if (assistant) return assistant

  const parts: string[] = []
  if (state.commands.length > 0) {
    parts.push(`${state.commands.length} command${state.commands.length === 1 ? '' : 's'}`)
  }
  if (state.filePaths.length > 0) {
    parts.push(`${state.filePaths.length} file${state.filePaths.length === 1 ? '' : 's'} changed`)
  }
  if (state.errors.length > 0) {
    parts.push(`${state.errors.length} error${state.errors.length === 1 ? '' : 's'}`)
  }
  return parts.join(' | ')
}

function buildLocalSummaryEvent(turnId: string, state: LocalTurnSummaryState): AppEvent {
  const commandExamples = state.commands.map(command => command.command).slice(0, 3)
  const filePathExamples = state.filePaths.slice(0, 5)
  const errorExamples = state.errors.slice(0, 5)
  const commandsIndexed = Math.min(state.commands.length, 50)
  const filePathsIndexed = Math.min(state.filePaths.length, 50)
  const errorsIndexed = Math.min(state.errors.length, 50)
  const summaryText = summarizeText(state)
  const primaryCommand = state.commands[state.commands.length - 1]?.command ?? null
  const primaryFilePath = state.filePaths[state.filePaths.length - 1] ?? null
  const primaryError = state.errors[state.errors.length - 1] ?? null

  return {
    type: 'SummaryNode',
    id: `summary-${turnId}`,
    turnId,
    nodeType: 'turn',
    status: summarizeStatus(state.finalStatus),
    summaryText,
    brief: {
      signal: summarizeSignal(state),
      agentMessage: state.assistantMessage,
      primaryCommand,
      primaryFilePath,
      primaryError,
    },
    counts: {
      commandsTotal: state.commands.length,
      commandsIndexed,
      commandsOmitted: state.commands.length - commandsIndexed,
      filePathsTotal: state.filePaths.length,
      filePathsIndexed,
      filePathsOmitted: state.filePaths.length - filePathsIndexed,
      errorsTotal: state.errors.length,
      errorsIndexed,
      errorsOmitted: state.errors.length - errorsIndexed,
    },
    digest: {
      commandExamples,
      filePathExamples,
      errorExamples,
    },
    lineage: {
      parentTurnId: state.parentTurnId,
      childTurnId: state.childTurnIds[state.childTurnIds.length - 1] ?? null,
      childTurnIds: [...state.childTurnIds],
      forkedFromThreadId: null,
      startedAfterRollback: false,
      wasRolledBack: state.finalStatus === 'cancelled',
    },
    evidence: {
      childTurnIds: [...state.childTurnIds],
      filePaths: [...state.filePaths],
      commands: [...state.commands],
      errors: [...state.errors],
    },
    ts: Date.now(),
  }
}

function resolveSummaryEndpoint(wsAddress: string, explicitBaseUrl?: string): string | null {
  const explicit = (explicitBaseUrl ?? '').trim()
  if (explicit) {
    const trimmed = explicit.endsWith('/') ? explicit.slice(0, -1) : explicit
    return trimmed.endsWith('/api/summarize') ? trimmed : `${trimmed}/api/summarize`
  }

  try {
    const parsed = new URL(wsAddress, window.location.origin)
    const protocol = parsed.protocol === 'wss:' ? 'https:' : parsed.protocol === 'ws:' ? 'http:' : parsed.protocol
    return `${protocol}//${parsed.host}/api/summarize`
  } catch {
    return null
  }
}

function ingestItemIntoLocalSummary(state: LocalTurnSummaryState, item: Record<string, unknown>) {
  if (item.type === 'commandExecution') {
    const command = ((item.command as string) ?? '').trim()
    const exitCode = (item.exitCode ?? null) as number | null
    if (command) state.commands.push({ command, exitCode })
    if (exitCode !== null && exitCode !== 0) {
      const errorText = firstString([
        item.error as string,
        item.stderr as string,
        item.aggregatedOutput as string,
        `Command failed: ${command}`,
      ])
      pushUnique(state.errors, errorText)
    }
    return
  }

  if (item.type === 'fileChange') {
    const changes = (item.changes as Array<{ path?: unknown }>) ?? []
    for (const change of changes) {
      if (typeof change.path === 'string') pushUnique(state.filePaths, change.path)
    }
    return
  }

  if (item.type === 'mcpToolCall' && item.status === 'failed') {
    const toolName = ((item.tool as string) ?? 'tool').trim()
    const errorText = firstString([item.error as string, `Tool failed: ${toolName}`])
    pushUnique(state.errors, errorText)
    return
  }

  if (item.type === 'agentMessage' || item.type === 'agent_message') {
    const text = ((item.text as string) ?? '').trim()
    const phase = item.phase as string | undefined
    if (text && (!phase || phase === 'final_answer')) {
      state.assistantMessage = text
    }
  }
}

function isInternalUserPrompt(text: string): boolean {
  const trimmed = text.trim()
  if (!trimmed) return true

  const lower = trimmed.toLowerCase()
  if (lower.startsWith('# agents.md instructions for')) return true
  if (lower.startsWith('<turn_aborted>')) return true
  if (lower.startsWith('<environment_context>')) return true
  if (lower.startsWith('<permissions instructions>')) return true
  if (lower.startsWith('<collaboration_mode>')) return true
  if (lower.startsWith('warning: apply_patch was requested via exec_command')) return true

  return false
}

// ---------------------------------------------------------------------------
// Map a real app-server ThreadItem (camelCase JSON) → our internal AppEvent
// ---------------------------------------------------------------------------
function normalizeItem(item: Record<string, unknown>, turnId: string): AppEvent | null {
  const id = item.id as string
  switch (item.type) {
    case 'commandExecution': {
      const exitCode = (item.exitCode ?? null) as number | null
      return {
        type: 'CommandExecution',
        id,
        turnId,
        cmd: (item.command as string) ?? '',
        exitCode,
        stdout: (item.aggregatedOutput as string) ?? '',
        stderr: '',
        ts: Date.now(),
      }
    }
    case 'fileChange': {
      const changes = (item.changes as Array<{ path: string; diff: string }>) ?? []
      const first = changes[0]
      return {
        type: 'PatchApply',
        id,
        turnId,
        filename: first?.path ?? '(unknown)',
        diff: first?.diff ?? '',
        status: item.status === 'completed' ? 'success' : 'error',
        ts: Date.now(),
      }
    }
    case 'mcpToolCall': {
      return {
        type: 'McpToolCall',
        id,
        turnId,
        toolName: (item.tool as string) ?? '',
        server: (item.server as string) ?? undefined,
        input: item.arguments,
        output: item.result,
        status: item.status === 'completed' ? 'success' : 'error',
        ts: Date.now(),
      }
    }
    case 'plan': {
      return {
        type: 'PlanUpdate',
        id,
        turnId,
        text: (item.text as string) ?? '',
        ts: Date.now(),
      }
    }
    case 'agentMessage':
    case 'agent_message': {
      const text = ((item.text as string) ?? '').trim()
      if (!text) return null
      const phase = item.phase as string | undefined
      if (phase && phase !== 'final_answer') return null
      return {
        type: 'AssistantMessage',
        id,
        turnId,
        text,
        ts: Date.now(),
      }
    }
    default:
      return null
  }
}

export function useAppServerWS(overrideUrl?: string) {
  const addEvent = useGraphStore(s => s.addEvent)
  const setWsStatus = useGraphStore(s => s.setWsStatus)
  const updateTurnLabel = useGraphStore(s => s.updateTurnLabel)
  const storeUrl = useGraphStore(s => s.wsUrl)
  const wsRef = useRef<WebSocket | null>(null)
  const backoffRef = useRef(1000)
  const unmountedRef = useRef(false)
  const localSummaryRef = useRef<Map<string, LocalTurnSummaryState>>(new Map())
  const turnOrderBySessionRef = useRef<Map<string, string[]>>(new Map())
  const summaryRequestVersionRef = useRef<Map<string, number>>(new Map())
  const url = overrideUrl ?? storeUrl
  const isReplayStyle = (address: string) => {
    try {
      const parsed = new URL(address, window.location.origin)
      return parsed.searchParams.has('file') || parsed.searchParams.has('watch')
    } catch {
      return false
    }
  }

  useEffect(() => {
    if (USE_MOCK) return  // skip WS entirely in mock mode
    if (!url) return       // no URL yet — wait for session selection
    const replayStyle = isReplayStyle(url)
    const summaryEndpoint = REMOTE_SUMMARY_ENABLED ? resolveSummaryEndpoint(url, SUMMARY_API_URL) : null
    localSummaryRef.current = new Map()
    turnOrderBySessionRef.current = new Map()
    summaryRequestVersionRef.current = new Map()

    unmountedRef.current = false

    function ensureTurnSummaryState(turnId: string, sessionId: string | null): LocalTurnSummaryState {
      const existing = localSummaryRef.current.get(turnId)
      if (existing) {
        if (!existing.sessionId && sessionId) existing.sessionId = sessionId
        return existing
      }
      const created: LocalTurnSummaryState = {
        sessionId,
        parentTurnId: null,
        childTurnIds: [],
        assistantMessage: null,
        commands: [],
        filePaths: [],
        errors: [],
        finalStatus: null,
        remoteSummary: false,
      }
      localSummaryRef.current.set(turnId, created)
      return created
    }

    function emitSummaryNode(turnId: string, state: LocalTurnSummaryState) {
      if (state.remoteSummary) return

      const localEvent = buildLocalSummaryEvent(turnId, state)
      addEvent(localEvent)

      if (!summaryEndpoint) return
      if (!state.finalStatus) return

      const nextVersion = (summaryRequestVersionRef.current.get(turnId) ?? 0) + 1
      summaryRequestVersionRef.current.set(turnId, nextVersion)

      const payload = {
        turnId,
        sessionId: state.sessionId,
        status: localEvent.status,
        parentTurnId: state.parentTurnId,
        childTurnIds: state.childTurnIds,
        assistantMessage: state.assistantMessage,
        commands: state.commands,
        filePaths: state.filePaths,
        errors: state.errors,
      }

      void fetch(summaryEndpoint, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
      })
        .then(async response => {
          if (!response.ok) return null
          const body = await response.json() as Record<string, unknown>
          return body
        })
        .then(body => {
          if (!body) return
          if (summaryRequestVersionRef.current.get(turnId) !== nextVersion) return
          const normalized = normalizeSummaryEvent({
            ...body,
            turnId,
          })
          if (normalized) addEvent(normalized)
        })
        .catch(() => {
          // local summary already emitted
        })
    }

    function connect() {
      if (unmountedRef.current) return
      setWsStatus('connecting')
      let wsConnectUrl = url
      try {
        const parsed = new URL(url, window.location.origin)
        if (parsed.protocol === 'http:') {
          parsed.protocol = 'ws:'
        } else if (parsed.protocol === 'https:') {
          parsed.protocol = 'wss:'
        }
        wsConnectUrl = parsed.toString()
      } catch {
        // Keep explicit WS URL if parsing fails.
      }
      const ws = new WebSocket(wsConnectUrl)
      wsRef.current = ws

      ws.onopen = () => {
        backoffRef.current = 1000
        setWsStatus('connected')
        // Skip handshake for replay connections (they start streaming immediately)
        if (!replayStyle) {
          ws.send(JSON.stringify({
            method: 'initialize',
            id: 0,
            params: { clientInfo: { name: 'agentcanvas_ui', version: '0.1.0' } },
          }))
          ws.send(JSON.stringify({ method: 'initialized', params: {} }))
        }
      }

      ws.onmessage = (ev) => {
        try {
          const msg = JSON.parse(ev.data as string) as {
            method?: string
            params?: Record<string, unknown>
          }
          const { method, params } = msg
          if (!method || !params) return

          if (method === 'thread/started') {
            const thread = params.thread as { id: string }
            addEvent({ type: 'ThreadStarted', threadId: thread.id, ts: Date.now() })

          } else if (method === 'turn/started') {
            const turn = params.turn as { id: string }
            const sessionId = (params.threadId as string) ?? null
            const turnState = ensureTurnSummaryState(turn.id, sessionId)
            const turnOrder = sessionId
              ? (turnOrderBySessionRef.current.get(sessionId) ?? [])
              : []
            const parentTurnId = turnOrder.length > 0 ? turnOrder[turnOrder.length - 1] : null
            if (parentTurnId) {
              turnState.parentTurnId = parentTurnId
              const parentState = localSummaryRef.current.get(parentTurnId)
              if (parentState) {
                pushUnique(parentState.childTurnIds, turn.id, 32)
                if (parentState.finalStatus) emitSummaryNode(parentTurnId, parentState)
              }
            }
            if (sessionId && !turnOrder.includes(turn.id)) {
              turnOrder.push(turn.id)
              turnOrderBySessionRef.current.set(sessionId, turnOrder)
            }

            addEvent({
              type: 'TurnStarted',
              turnId: turn.id,
              sessionId: params.threadId as string,
              userPrompt: '(typing…)',
              ts: Date.now(),
            })

          } else if (method === 'turn/completed') {
            const turn = params.turn as { id: string; status: string }
            const status: TurnStatus =
              turn.status === 'completed' ? 'success'
              : turn.status === 'failed' ? 'error'
              : 'cancelled'
            addEvent({ type: 'TurnComplete', turnId: turn.id, status, ts: Date.now() })
            const sessionId = typeof params.threadId === 'string' ? params.threadId : null
            const state = ensureTurnSummaryState(turn.id, sessionId)
            state.finalStatus = status
            emitSummaryNode(turn.id, state)

          } else if (method === 'agentcanvas/summaryNode') {
            const summaryEvent = normalizeSummaryEvent(asRecord(params) ?? {})
            if (summaryEvent) {
              const state = ensureTurnSummaryState(summaryEvent.turnId, typeof params.threadId === 'string' ? params.threadId : null)
              state.remoteSummary = true
              addEvent(summaryEvent)
            }

          } else if (method === 'item/completed') {
            const item = params.item as Record<string, unknown>
            const turnId = params.turnId as string
            const state = ensureTurnSummaryState(turnId, typeof params.threadId === 'string' ? params.threadId : null)
            ingestItemIntoLocalSummary(state, item)

            // userMessage → extract prompt text and retroactively update turn label
            if (item.type === 'userMessage') {
              const content = item.content as Array<{ type: string; text: string }>
              const text = (content?.find(c => c.type === 'text')?.text ?? '').trim()
              if (text && !isInternalUserPrompt(text)) updateTurnLabel(turnId, text)
              return
            }

            const normalized = normalizeItem(item, turnId)
            if (normalized) addEvent(normalized)
          }
        } catch {
          // malformed message — skip
        }
      }

      ws.onclose = () => {
        if (unmountedRef.current) return
        setWsStatus('disconnected')
        // Don't auto-reconnect for replay sessions
        if (!replayStyle) {
          const delay = backoffRef.current
          backoffRef.current = Math.min(delay * 2, MAX_BACKOFF)
          setTimeout(connect, delay)
        }
      }

      ws.onerror = () => ws.close()
    }

    connect()

    return () => {
      unmountedRef.current = true
      wsRef.current?.close()
    }
  }, [url])
}
