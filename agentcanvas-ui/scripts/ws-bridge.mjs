#!/usr/bin/env node
/**
 * AgentCanvas WebSocket Bridge
 *
 * Wraps `codex proto` (stdio transport) and exposes it as a WebSocket
 * server on port 8080. The AgentCanvas UI connects here and observes
 * all Codex session events in real-time.
 *
 * Usage:
 *   node scripts/ws-bridge.mjs [--port 8080] [--thread-id thr_xxx]
 *
 * Then:
 *   1. Open http://localhost:5173 in your browser
 *   2. Click "Connect to Codex session" and enter ws://localhost:8080
 *   3. Type messages in THIS terminal to send turns to Codex
 *   4. Watch the graph update live in the browser
 */

import { createServer } from 'http'
import { WebSocketServer } from 'ws'
import { spawn } from 'child_process'
import * as readline from 'readline'

// ── Parse CLI args ────────────────────────────────────────────────────────────
const args = process.argv.slice(2)
const portArg = args[args.indexOf('--port') + 1]
const threadIdArg = args[args.indexOf('--thread-id') + 1]
const PORT = portArg ? parseInt(portArg) : 8080

// ── State ─────────────────────────────────────────────────────────────────────
let nextId = 1
let threadId = threadIdArg ?? null
let turnId = null
const clients = new Set()

// ── Spawn codex proto ─────────────────────────────────────────────────────────
console.log('[bridge] spawning codex proto…')
const codex = spawn('codex', ['proto'], {
  stdio: ['pipe', 'pipe', 'inherit'],
})

codex.on('error', (err) => {
  console.error('[bridge] failed to spawn codex:', err.message)
  console.error('[bridge] make sure `codex` is in your PATH')
  process.exit(1)
})

codex.on('exit', (code) => {
  console.log(`[bridge] codex exited with code ${code}`)
  process.exit(code ?? 0)
})

// ── Send JSON-RPC to codex ────────────────────────────────────────────────────
function send(msg) {
  const line = JSON.stringify(msg) + '\n'
  codex.stdin.write(line)
}

// ── Broadcast to all UI clients ───────────────────────────────────────────────
function broadcast(msg) {
  const text = typeof msg === 'string' ? msg : JSON.stringify(msg)
  for (const ws of clients) {
    if (ws.readyState === 1 /* OPEN */) ws.send(text)
  }
}

// ── Handle lines from codex stdout ───────────────────────────────────────────
const rl = readline.createInterface({ input: codex.stdout })

rl.on('line', (line) => {
  if (!line.trim()) return
  let msg
  try { msg = JSON.parse(line) } catch { return }

  // Forward every notification to the UI
  broadcast(msg)

  // Track thread/started so we know the threadId
  if (msg.method === 'thread/started') {
    threadId = msg.params?.thread?.id ?? threadId
    console.log(`[bridge] thread started: ${threadId}`)
  }

  // Track turn/started
  if (msg.method === 'turn/started') {
    turnId = msg.params?.turn?.id ?? null
  }

  // Handle initialize response → auto-send initialized + thread/start
  if (msg.id === 1 && msg.result) {
    console.log('[bridge] initialized. starting thread…')
    send({ method: 'initialized' })

    if (threadId) {
      console.log(`[bridge] resuming thread ${threadId}`)
      send({ method: 'thread/resume', id: nextId++, params: { threadId } })
    } else {
      send({ method: 'thread/start', id: nextId++, params: {} })
    }
  }
})

// ── WebSocket server ──────────────────────────────────────────────────────────
const server = createServer()
const wss = new WebSocketServer({ server })

wss.on('connection', (ws) => {
  clients.add(ws)
  console.log(`[bridge] UI connected (${clients.size} total)`)

  // Send the current thread state so UI can catch up if connecting late
  if (threadId) {
    ws.send(JSON.stringify({
      method: 'thread/started',
      params: { thread: { id: threadId } },
    }))
  }

  ws.on('close', () => {
    clients.delete(ws)
    console.log(`[bridge] UI disconnected (${clients.size} remaining)`)
  })
})

server.listen(PORT, '127.0.0.1', () => {
  console.log(`[bridge] WebSocket server listening on ws://127.0.0.1:${PORT}`)
})

// ── Read turns from stdin ─────────────────────────────────────────────────────
const stdinRl = readline.createInterface({ input: process.stdin })

stdinRl.on('line', (line) => {
  const text = line.trim()
  if (!text || !threadId) {
    if (!threadId) console.log('[bridge] waiting for thread to start…')
    return
  }

  console.log(`[bridge] sending turn: "${text}"`)
  send({
    method: 'turn/start',
    id: nextId++,
    params: {
      threadId,
      input: [{ type: 'text', text }],
    },
  })
})

stdinRl.on('close', () => process.exit(0))

// ── Initialize codex ──────────────────────────────────────────────────────────
// Must be sent first, before any other request
send({
  method: 'initialize',
  id: 1,
  params: {
    clientInfo: {
      name: 'agentcanvas',
      title: 'AgentCanvas Graph UI',
      version: '0.1.0',
    },
  },
})

console.log('[bridge] sent initialize. waiting for codex…')
console.log('[bridge] open http://localhost:5173 and connect to ws://127.0.0.1:8080')
console.log('[bridge] type a message below and press Enter to start a turn:\n')
