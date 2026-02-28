import { useEffect, useRef } from 'react'
import { useGraphStore } from '../store/graphStore'
import type { AppEvent } from '../lib/types'

const WS_URL = import.meta.env.VITE_WS_URL ?? 'ws://localhost:5173/ws'
const MAX_BACKOFF = 30_000

export function useAppServerWS() {
  const addEvent = useGraphStore(s => s.addEvent)
  const setWsStatus = useGraphStore(s => s.setWsStatus)
  const wsRef = useRef<WebSocket | null>(null)
  const backoffRef = useRef(1000)
  const unmountedRef = useRef(false)

  useEffect(() => {
    unmountedRef.current = false

    function connect() {
      if (unmountedRef.current) return
      setWsStatus('connecting')
      const ws = new WebSocket(WS_URL)
      wsRef.current = ws

      ws.onopen = () => {
        backoffRef.current = 1000
        setWsStatus('connected')
        // Send initialize handshake
        ws.send(JSON.stringify({
          method: 'initialize',
          id: 0,
          params: { clientInfo: { name: 'agentcanvas_ui', version: '0.1.0' } },
        }))
        ws.send(JSON.stringify({ method: 'initialized', params: {} }))
      }

      ws.onmessage = (ev) => {
        try {
          const msg = JSON.parse(ev.data as string)
          // JSON-RPC notification: { method, params }
          const event = (msg.params ?? msg) as AppEvent
          if (event?.type) addEvent(event)
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
  }, [])
}
