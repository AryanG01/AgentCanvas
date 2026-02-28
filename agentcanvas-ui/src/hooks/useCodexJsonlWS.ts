import { useEffect, useRef } from 'react'
import { useGraphStore } from '../store/graphStore'
import type { AppEvent } from '../lib/types'

const MAX_BACKOFF = 30_000

// ---------------------------------------------------------------------------
// JSONL Event Types (from codex-rs/exec/src/exec_events.rs)
// ---------------------------------------------------------------------------

interface JsonlThreadStarted {
  type: 'thread.started'
  thread_id: string
}

interface JsonlTurnStarted {
  type: 'turn.started'
}

interface JsonlTurnCompleted {
  type: 'turn.completed'
  usage?: {
    input_tokens: number
    cached_input_tokens: number
    output_tokens: number
  }
}

interface JsonlTurnFailed {
  type: 'turn.failed'
  error: {
    message: string
  }
}

interface JsonlThreadItem {
  id: string
  type: string
  [key: string]: unknown
}

interface JsonlItemStarted {
  type: 'item.started'
  item: JsonlThreadItem
}

interface JsonlItemUpdated {
  type: 'item.updated'
  item: JsonlThreadItem
}

interface JsonlItemCompleted {
  type: 'item.completed'
  item: JsonlThreadItem
}

type JsonlEvent =
  | JsonlThreadStarted
  | JsonlTurnStarted
  | JsonlTurnCompleted
  | JsonlTurnFailed
  | JsonlItemStarted
  | JsonlItemUpdated
  | JsonlItemCompleted

// ---------------------------------------------------------------------------
// Convert JSONL item to AppEvent
// ---------------------------------------------------------------------------

function normalizeItem(item: JsonlThreadItem, turnId: string): AppEvent | null {
  const id = item.id
  const itemType = item.type

  switch (itemType) {
    case 'command_execution': {
      const exitCode = (item.exit_code ?? null) as number | null
      return {
        type: 'CommandExecution',
        id,
        turnId,
        cmd: (item.command as string) ?? '',
        exitCode,
        stdout: (item.aggregated_output as string) ?? '',
        stderr: '',
        ts: Date.now(),
      }
    }

    case 'file_change': {
      const changes = (item.changes as Array<{ path: string; diff: string }>) ?? []
      const first = changes[0]
      const status = item.status === 'completed' ? 'success' : 'error'
      return {
        type: 'PatchApply',
        id,
        turnId,
        filename: first?.path ?? '(unknown)',
        diff: first?.diff ?? '',
        status: status as 'success' | 'error',
        ts: Date.now(),
      }
    }

    case 'mcp_tool_call': {
      const status = item.status === 'completed' ? 'success' : 'error'
      return {
        type: 'McpToolCall',
        id,
        turnId,
        toolName: (item.tool as string) ?? '',
        server: (item.server as string) ?? undefined,
        input: item.arguments,
        output: item.result,
        status: status as 'success' | 'error',
        ts: Date.now(),
      }
    }

    case 'todo_list': {
      const items = (item.items as Array<{ content: string }>) ?? []
      const text = items.map(i => `- ${i.content}`).join('\n')
      return {
        type: 'PlanUpdate',
        id,
        turnId,
        text,
        ts: Date.now(),
      }
    }

    case 'agent_message': {
      // Agent messages contain user prompt text
      // We use this to retroactively update the turn label
      const text = (item.text as string) ?? ''
      return {
        type: 'UserPromptPatch',
        turnId,
        userPrompt: text,
        ts: Date.now(),
      }
    }

    default:
      // Unsupported item type — skip it
      return null
  }
}

// ---------------------------------------------------------------------------
// Hook implementation
// ---------------------------------------------------------------------------

export function useCodexJsonlWS(overrideUrl?: string | null) {
  const addEvent = useGraphStore(s => s.addEvent)
  const setWsStatus = useGraphStore(s => s.setWsStatus)
  const updateTurnLabel = useGraphStore(s => s.updateTurnLabel)
  const storeUrl = useGraphStore(s => s.jsonlWsUrl)
  const wsRef = useRef<WebSocket | null>(null)
  const backoffRef = useRef(1000)
  const unmountedRef = useRef(false)
  const currentTurnId = useRef<string | null>(null)
  const currentThreadId = useRef<string | null>(null)
  const url = overrideUrl === null ? null : (overrideUrl ?? storeUrl)

  useEffect(() => {
    if (url === null) return  // Disabled
    if (!url) return // no URL yet

    unmountedRef.current = false

    function connect() {
      if (unmountedRef.current) return
      setWsStatus('connecting')
      const ws = new WebSocket(url)
      wsRef.current = ws

      ws.onopen = () => {
        backoffRef.current = 1000
        setWsStatus('connected')
      }

      ws.onmessage = (ev) => {
        try {
          const event = JSON.parse(ev.data as string) as JsonlEvent
          const eventType = event.type

          if (eventType === 'thread.started') {
            const threadId = event.thread_id
            currentThreadId.current = threadId
            addEvent({
              type: 'ThreadStarted',
              threadId,
              ts: Date.now(),
            })
          } else if (eventType === 'turn.started') {
            // Generate unique turn ID
            const turnId = `turn_${Date.now()}_${Math.random().toString(36).slice(2, 9)}`
            currentTurnId.current = turnId
            addEvent({
              type: 'TurnStarted',
              turnId,
              sessionId: currentThreadId.current ?? 'unknown',
              userPrompt: '(typing…)',
              ts: Date.now(),
            })
          } else if (eventType === 'turn.completed') {
            if (currentTurnId.current) {
              addEvent({
                type: 'TurnComplete',
                turnId: currentTurnId.current,
                status: 'success',
                ts: Date.now(),
              })
            }
          } else if (eventType === 'turn.failed') {
            if (currentTurnId.current) {
              addEvent({
                type: 'TurnComplete',
                turnId: currentTurnId.current,
                status: 'error',
                ts: Date.now(),
              })
            }
          } else if (eventType === 'item.completed') {
            if (!currentTurnId.current) return

            const item = event.item
            const normalized = normalizeItem(item, currentTurnId.current)

            if (normalized) {
              // Special handling for user prompt patches
              if (normalized.type === 'UserPromptPatch') {
                updateTurnLabel(normalized.turnId, normalized.userPrompt)
              } else {
                addEvent(normalized)
              }
            }
          }
          // item.started and item.updated are ignored — we wait for completion
        } catch (err) {
          console.error('Failed to parse JSONL event:', err)
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
  }, [url, addEvent, setWsStatus, updateTurnLabel])
}
