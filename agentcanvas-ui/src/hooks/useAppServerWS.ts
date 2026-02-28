import { useEffect, useRef } from 'react'
import { useGraphStore } from '../store/graphStore'
import type { AppEvent } from '../lib/types'

const MAX_BACKOFF = 30_000
const USE_MOCK = import.meta.env.VITE_USE_MOCK === 'true'

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
  const url = overrideUrl ?? storeUrl

  useEffect(() => {
    if (USE_MOCK) return  // skip WS entirely in mock mode

    unmountedRef.current = false

    function connect() {
      if (unmountedRef.current) return
      setWsStatus('connecting')
      const ws = new WebSocket(url)
      wsRef.current = ws

      ws.onopen = () => {
        backoffRef.current = 1000
        setWsStatus('connected')
        // Initialize handshake
        ws.send(JSON.stringify({
          method: 'initialize',
          id: 0,
          params: { clientInfo: { name: 'agentcanvas_ui', version: '0.1.0' } },
        }))
        ws.send(JSON.stringify({ method: 'initialized', params: {} }))
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
            addEvent({
              type: 'TurnStarted',
              turnId: turn.id,
              sessionId: params.threadId as string,
              userPrompt: '(typing…)',
              ts: Date.now(),
            })

          } else if (method === 'turn/completed') {
            const turn = params.turn as { id: string; status: string }
            const status =
              turn.status === 'completed' ? 'success'
              : turn.status === 'failed' ? 'error'
              : 'cancelled'
            addEvent({ type: 'TurnComplete', turnId: turn.id, status, ts: Date.now() })

          } else if (method === 'item/completed') {
            const item = params.item as Record<string, unknown>
            const turnId = params.turnId as string

            // userMessage → extract prompt text and retroactively update turn label
            if (item.type === 'userMessage') {
              const content = item.content as Array<{ type: string; text: string }>
              const text = content?.find(c => c.type === 'text')?.text ?? ''
              if (text) updateTurnLabel(turnId, text)
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
        const delay = backoffRef.current
        backoffRef.current = Math.min(delay * 2, MAX_BACKOFF)
        setTimeout(connect, delay)
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
