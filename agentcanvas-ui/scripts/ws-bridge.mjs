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
import { existsSync, readFileSync } from 'fs'
import { homedir } from 'os'
import { dirname, join } from 'path'
import { fileURLToPath } from 'url'
import * as readline from 'readline'

// Load .env.local from repo root (Node doesn't read Vite env files)
const __dirname = dirname(fileURLToPath(import.meta.url))
for (const envFile of ['.env.local', '.env']) {
  for (const base of [join(__dirname, '..'), join(__dirname, '..', '..')]) {
    const envPath = join(base, envFile)
    if (existsSync(envPath)) {
      for (const line of readFileSync(envPath, 'utf-8').split('\n')) {
        const trimmed = line.trim()
        if (!trimmed || trimmed.startsWith('#')) continue
        const eq = trimmed.indexOf('=')
        if (eq === -1) continue
        const key = trimmed.slice(0, eq).trim()
        let val = trimmed.slice(eq + 1).trim()
        if ((val.startsWith('"') && val.endsWith('"')) || (val.startsWith("'") && val.endsWith("'"))) {
          val = val.slice(1, -1)
        }
        if (!process.env[key]) process.env[key] = val
      }
    }
  }
}

// ── Parse CLI args ────────────────────────────────────────────────────────────
const args = process.argv.slice(2)
const portArg = args[args.indexOf('--port') + 1]
const threadIdArg = args[args.indexOf('--thread-id') + 1]
const PORT = portArg ? parseInt(portArg) : 8080
const SUMMARY_DIR = join(homedir(), '.codex', 'agentcanvas', 'summaries')
const MAX_SYNTHETIC_SUMMARY_CHARS = 400

function sanitizePathComponent(value) {
  return String(value).replace(/[^A-Za-z0-9._-]/g, '_')
}

function normalizeSummaryText(value) {
  return String(value ?? '')
    .replace(/\s+/g, ' ')
    .trim()
    .slice(0, MAX_SYNTHETIC_SUMMARY_CHARS)
}

function makeFallbackSummaryNode(threadId, turnId, status, summaryText = '') {
  const normalized = normalizeSummaryText(summaryText)
  // Build a human-readable summary: prefer the agent message, fall back to status
  const agentSnippet = normalized ? normalized.slice(0, 120) : null
  const displaySummary = agentSnippet
    ? `Outcome: ${status}. Agent: ${agentSnippet}`
    : `Outcome: ${status}.`

  return {
    schema_version: 'agentcanvas.turn.v2',
    summary_kind: 'agentcanvas_turn_summary',
    thread_id: threadId,
    session_id: turnId,
    forked_from_thread_id: null,
    nodes: [
      {
        node_id: `turn:${turnId}`,
        parent_id: null,
        node_type: 'turn',
        title: `Turn ${turnId}`,
        summary: displaySummary,
        status,
        brief: {
          signal: status === 'interrupted' ? 'error' : 'status_only',
          agent_message: agentSnippet,
          primary_command: null,
          primary_file_path: null,
          primary_error: status === 'interrupted' ? (agentSnippet || 'interrupted') : null,
        },
        lineage: {
          parent_turn_id: null,
          forked_from_thread_id: null,
          started_after_rollback: false,
        },
        counts: {
          commands_total: 0,
          commands_indexed: 0,
          commands_omitted: 0,
          file_paths_total: 0,
          file_paths_indexed: 0,
          file_paths_omitted: 0,
          errors_total: 0,
          errors_indexed: 0,
          errors_omitted: 0,
        },
        digest: {
          command_examples: [],
          file_path_examples: [],
          error_examples: [],
        },
        evidence: {
          file_paths: [],
          commands: [],
          errors: [],
        },
      },
    ],
  }
}

function readTurnSummaryNode(threadIdValue, turnIdValue) {
  if (!threadIdValue || !turnIdValue) return null
  const summaryPath = join(
    SUMMARY_DIR,
    sanitizePathComponent(threadIdValue),
    `${sanitizePathComponent(turnIdValue)}.json`,
  )
  try {
    const summary = JSON.parse(readFileSync(summaryPath, 'utf-8'))
    if (!Array.isArray(summary?.nodes)) return null
    const exactNodeId = `turn:${turnIdValue}`
    return (
      summary.nodes.find((entry) => entry?.node_id === exactNodeId)
      ?? summary.nodes.find((entry) => entry?.node_type === 'turn')
      ?? null
    )
  } catch {
    return null
  }
}

function extractAgentMessage(item) {
  if (!item || typeof item !== 'object') return null
  // Handle normalized format
  if (item.type === 'agentMessage' && typeof item.text === 'string') return item.text
  // Handle protocol format: message item with role=assistant and text content
  if (item.type === 'message' && item.role === 'assistant') {
    const content = Array.isArray(item.content) ? item.content : []
    const textPart = content.find((c) => c?.type === 'output_text' || c?.type === 'text')
    if (textPart?.text) return textPart.text
  }
  // Handle codex proto format: response items with content array
  if (Array.isArray(item.content)) {
    for (const part of item.content) {
      if (part?.type === 'output_text' && typeof part.text === 'string') return part.text
      if (part?.type === 'text' && typeof part.text === 'string') return part.text
    }
  }
  // Handle simple message field (codex proto agent_message event)
  if (typeof item.message === 'string' && item.message.trim()) return item.message
  return null
}

// ── State ─────────────────────────────────────────────────────────────────────
let nextId = 1
let threadId = threadIdArg ?? null
let turnId = null
const turnCompletedAgentMessages = new Map()
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

  if (msg.method === 'turn/completed') {
    const completedTurnId = msg.params?.turn?.id ?? turnId
    const status = msg.params?.turn?.status ?? 'completed'
    // The turn/completed event may carry last_agent_message directly (from the protocol)
    const turnLastMessage = msg.params?.turn?.lastAgentMessage
      ?? msg.params?.turn?.last_agent_message
      ?? msg.params?.lastAgentMessage
      ?? msg.params?.last_agent_message
      ?? null
    const messageText = turnLastMessage
      ?? (completedTurnId ? turnCompletedAgentMessages.get(completedTurnId) : null)
    if (completedTurnId) {
      turnCompletedAgentMessages.delete(completedTurnId)
    }
    // The recorder persists summaries asynchronously after emitting turn/completed.
    // Retry reading the persisted summary with a short delay to avoid the race.
    const emitSummary = (node) => {
      if (!node) return
      broadcast({
        method: 'agentcanvas/summaryNode',
        params: {
          threadId: threadId ?? '',
          turnId: completedTurnId ?? '',
          node,
        },
      })
    }
    const persisted = readTurnSummaryNode(threadId, completedTurnId)
    if (persisted) {
      emitSummary(persisted)
    } else {
      // Emit an immediate fallback so the UI isn't empty
      const fallback = makeFallbackSummaryNode(threadId ?? '', completedTurnId, status, messageText ?? '')
      emitSummary(fallback?.nodes?.[0] ?? fallback)
      // Then retry after a delay to pick up the persisted (potentially LLM-enhanced) summary
      setTimeout(() => {
        const delayed = readTurnSummaryNode(threadId, completedTurnId)
        if (delayed) emitSummary(delayed)
      }, 3000)
    }
  }

  if (msg.method === 'item/completed') {
    const item = msg.params?.item
    const itemTurnId = msg.params?.turnId ?? turnId
    const text = extractAgentMessage(item)
    if (itemTurnId && text) {
      turnCompletedAgentMessages.set(itemTurnId, text)
    }
  }

  // Also capture agent messages from codex/event/* notifications (proto transport)
  if (msg.method?.startsWith('codex/event/')) {
    const eventMsg = msg.params?.msg ?? msg.params
    // task_complete carries last_agent_message
    if (eventMsg?.last_agent_message && turnId) {
      turnCompletedAgentMessages.set(turnId, eventMsg.last_agent_message)
    }
    // Raw response items may contain agent message text
    if (eventMsg?.item) {
      const text = extractAgentMessage(eventMsg.item)
      if (turnId && text) turnCompletedAgentMessages.set(turnId, text)
    }
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
