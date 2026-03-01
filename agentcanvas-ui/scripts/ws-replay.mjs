#!/usr/bin/env node

/**
 * ws-replay.mjs — Replay historical Codex rollout sessions over WebSocket.
 *
 * Reads rollout `.jsonl` files from ~/.codex/sessions/ and serves them over
 * WebSocket using the same JSON-RPC protocol the app-server UI speaks.
 *
 * Also serves GET /api/sessions — JSON list of available sessions.
 *
 * Usage:
 *   node scripts/ws-replay.mjs [--port 8080] [--speed 10]
 *   node scripts/ws-replay.mjs <rollout-file.jsonl> [--port 8080] [--speed 10]
 *   node scripts/ws-replay.mjs --latest [--port 8080]
 *
 * The UI can connect with ws://localhost:8080?file=<path> to replay a specific
 * session. Without ?file=, replays the CLI-specified or latest file.
 */

import { existsSync, readFileSync, readdirSync } from "node:fs";
import { createServer } from "node:http";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { URL } from "node:url";
import { WebSocketServer } from "ws";

// Load .env.local from repo root (Node doesn't read Vite env files)
const __dirname = dirname(fileURLToPath(import.meta.url));
for (const envFile of [".env.local", ".env"]) {
  // Check both agentcanvas-ui/ and repo root
  for (const base of [join(__dirname, ".."), join(__dirname, "..", "..")]) {
    const envPath = join(base, envFile);
    if (existsSync(envPath)) {
      for (const line of readFileSync(envPath, "utf-8").split("\n")) {
        const trimmed = line.trim();
        if (!trimmed || trimmed.startsWith("#")) continue;
        const eq = trimmed.indexOf("=");
        if (eq === -1) continue;
        const key = trimmed.slice(0, eq).trim();
        let val = trimmed.slice(eq + 1).trim();
        // Strip surrounding quotes
        if ((val.startsWith('"') && val.endsWith('"')) || (val.startsWith("'") && val.endsWith("'"))) {
          val = val.slice(1, -1);
        }
        if (!process.env[key]) process.env[key] = val;
      }
    }
  }
}

// ---------------------------------------------------------------------------
// CLI argument parsing
// ---------------------------------------------------------------------------

const args = process.argv.slice(2);

function flag(name) {
  const i = args.indexOf(name);
  if (i === -1) return undefined;
  return args[i + 1];
}

function hasFlag(name) {
  return args.includes(name);
}

const PORT = Number(flag("--port") ?? 8080);
const SPEED = Number(flag("--speed") ?? 10);
const useLatest = hasFlag("--latest");

let defaultRolloutPath = args.find(
  (a) => !a.startsWith("--") && a !== flag("--port") && a !== flag("--speed"),
);

// ---------------------------------------------------------------------------
// Session discovery
// ---------------------------------------------------------------------------

const SESSIONS_DIR = join(homedir(), ".codex", "sessions");
const SUMMARY_DIR = join(homedir(), ".codex", "agentcanvas", "summaries");
const MAX_SYNTHETIC_SUMMARY_CHARS = 400;
const SUMMARY_LLM_MODEL = "gpt-4.1-nano";
const SUMMARY_LLM_MAX_TOKENS = 2048;
const SUMMARY_LLM_TIMEOUT_MS = 15_000;

const SUMMARY_SYSTEM_PROMPT = `\
You summarize coding-agent turns into action-oriented prose.
Lead with the primary action verb: Edited, Ran, Fixed, Searched, Debugged, Read, Created, Deleted, Installed, Built, Tested.
Include specific files/commands when they are the point of the turn.
Mention errors prominently if they occurred.
Return a JSON array where each item has:
  "turn_id": the turn identifier,
  "short_summary": action-oriented label, max 80 chars (for graph node display),
  "detail_summary": expanded detail, max 250 chars (for evidence panel).
Examples:
  short_summary: "Explored project structure via pwd, ls, find"
  detail_summary: "Ran 4 commands to locate portfolio project. Used find with rg to search directories, confirmed working directory with pwd and ls."
  short_summary: "Edited 2 test files, fixed assertion"
  detail_summary: "Fixed failing test in src/lib.test.ts by updating expected value. Also edited src/utils.test.ts to add missing import."
Do not invent facts. Prioritize errors and failures over status text.
Return ONLY JSON, no markdown fences, no extra text.`;

function sanitizePathComponent(value) {
  return String(value).replace(/[^A-Za-z0-9._-]/g, "_");
}

function normalizeSummaryText(value) {
  return String(value ?? "")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, MAX_SYNTHETIC_SUMMARY_CHARS);
}

function makeFallbackSummaryNode(threadId, turnId, status, parentTurnId = null, summaryText = "") {
  const normalized = normalizeSummaryText(summaryText);
  const brief = normalized || `${status} turn`;

  return {
    schema_version: "agentcanvas.turn.v2",
    summary_kind: "agentcanvas_turn_summary",
    thread_id: threadId,
    session_id: turnId,
    forked_from_thread_id: null,
    nodes: [
      {
        node_id: `turn:${turnId}`,
        parent_id: parentTurnId ? `turn:${parentTurnId}` : null,
        node_type: "turn",
        title: `Turn ${turnId}`,
        summary: brief,
        status,
        brief: {
          signal: status === "interrupted" ? "error" : "status_only",
          agent_message: brief,
          primary_command: null,
          primary_file_path: null,
          primary_error: status === "interrupted" ? brief : null,
        },
        lineage: {
          parent_turn_id: parentTurnId,
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
  };
}

function resolveSummaryNode(threadIdValue, turnIdValue, status, parentTurnId, lastAgentMessage, fallbackCache) {
  const persistedSummary = readTurnSummaryNode(threadIdValue, turnIdValue);
  if (persistedSummary) {
    return persistedSummary;
  }

  if (fallbackCache?.has(turnIdValue)) {
    return fallbackCache.get(turnIdValue);
  }

  const fallback = makeFallbackSummaryNode(
    threadIdValue,
    turnIdValue,
    status,
    parentTurnId,
    lastAgentMessage,
  );
  // makeFallbackSummaryNode returns the full envelope {schema_version, nodes: [...]},
  // but the UI expects the inner node object {node_id, summary, brief, ...}
  return fallback?.nodes?.[0] ?? fallback;
}

function readTurnSummaryNode(threadId, turnId) {
  if (!threadId || !turnId) return null;
  const summaryPath = join(
    SUMMARY_DIR,
    sanitizePathComponent(threadId),
    `${sanitizePathComponent(turnId)}.json`,
  );
  try {
    const summary = JSON.parse(readFileSync(summaryPath, "utf-8"));
    if (!Array.isArray(summary?.nodes)) return null;
    const exactNodeId = `turn:${turnId}`;
    const node =
      summary.nodes.find((entry) => entry?.node_id === exactNodeId)
      ?? summary.nodes.find((entry) => entry?.node_type === "turn")
      ?? summary.nodes[0];
    if (!node || typeof node !== "object") return null;
    return node;
  } catch {
    return null;
  }
}

function findAllRollouts(dir) {
  const results = [];
  try {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const full = join(dir, entry.name);
      if (entry.isDirectory()) {
        results.push(...findAllRollouts(full));
      } else if (entry.name.startsWith("rollout-") && entry.name.endsWith(".jsonl")) {
        results.push(full);
      }
    }
  } catch {
    // directory doesn't exist or not readable
  }
  return results;
}

function normalizeSessionTitle(value) {
  if (typeof value !== "string") return null;
  const compact = value.replace(/\s+/g, " ").trim();
  if (!compact) return null;
  return compact.slice(0, 140);
}

function extractTextFromContent(content) {
  if (typeof content === "string") return normalizeSessionTitle(content);
  if (!Array.isArray(content)) return null;

  for (const part of content) {
    if (typeof part === "string") {
      const normalized = normalizeSessionTitle(part);
      if (normalized) return normalized;
      continue;
    }
    if (!part || typeof part !== "object") continue;

    const text =
      (typeof part.text === "string" && part.text)
      || (typeof part.input_text === "string" && part.input_text)
      || (typeof part.content === "string" && part.content)
      || null;
    const normalized = normalizeSessionTitle(text);
    if (normalized) return normalized;
  }

  return null;
}

function extractFirstUserPrompt(obj) {
  if (!obj || typeof obj !== "object") return null;

  const payload = obj.payload && typeof obj.payload === "object" ? obj.payload : {};

  if (obj.type === "event_msg") {
    if (payload.type === "user_message") {
      return normalizeSessionTitle(payload.message);
    }
    if (payload.type === "task_started") {
      return (
        normalizeSessionTitle(payload.user_prompt)
        || normalizeSessionTitle(payload.user_input)
        || normalizeSessionTitle(payload.prompt)
        || normalizeSessionTitle(payload.input)
        || normalizeSessionTitle(payload.message)
      );
    }
  }

  if (obj.type === "turn_context") {
    return (
      normalizeSessionTitle(payload.user_prompt)
      || normalizeSessionTitle(payload.userPrompt)
      || normalizeSessionTitle(payload.user_input)
      || normalizeSessionTitle(payload.input)
      || normalizeSessionTitle(payload.prompt)
      || extractTextFromContent(payload.messages)
    );
  }

  if (obj.type === "response_item") {
    if (payload.type === "user_message") {
      return (
        normalizeSessionTitle(payload.message)
        || normalizeSessionTitle(payload.text)
        || extractTextFromContent(payload.content)
      );
    }
    if (payload.type === "message" && payload.role === "user") {
      return extractTextFromContent(payload.content);
    }
  }

  return null;
}

function readSessionInfo(filePath) {
  try {
    const content = readFileSync(filePath, "utf-8");
    const lines = content.split("\n").filter((l) => l.trim());

    let meta = null;
    let taskStartedCount = 0;
    let turnContextCount = 0;
    let firstPrompt = null;

    for (const line of lines) {
      try {
        const obj = JSON.parse(line);
        if (obj.type === "session_meta" && !meta) meta = obj.payload;
        if (obj.type === "event_msg" && obj.payload?.type === "task_started") taskStartedCount++;
        if (obj.type === "turn_context") turnContextCount++;
        if (!firstPrompt) firstPrompt = extractFirstUserPrompt(obj);
      } catch {
        // skip
      }
    }

    if (!meta) return null;

    const ts = new Date(meta.timestamp);
    const turns = taskStartedCount > 0 ? taskStartedCount : turnContextCount;

    return {
      date: ts.toLocaleDateString("en-CA", { timeZone: "Asia/Singapore" }),
      time: ts.toLocaleTimeString("en-GB", { timeZone: "Asia/Singapore", hour: "2-digit", minute: "2-digit" }),
      turns,
      source: meta.source || "unknown",
      cwd: meta.cwd || "",
      file: filePath,
      id: meta.id,
      title: firstPrompt,
    };
  } catch {
    return null;
  }
}

function getSessionList() {
  const allRollouts = findAllRollouts(SESSIONS_DIR).sort().reverse();
  const sessions = [];
  for (const f of allRollouts) {
    const info = readSessionInfo(f);
    if (info) sessions.push(info);
  }
  return sessions;
}

// Resolve default rollout
if (useLatest && !defaultRolloutPath) {
  const all = findAllRollouts(SESSIONS_DIR).sort();
  if (all.length > 0) {
    defaultRolloutPath = all[all.length - 1];
    console.log("Default rollout (latest):", defaultRolloutPath);
  }
}

if (defaultRolloutPath) {
  defaultRolloutPath = resolve(defaultRolloutPath);
}

// ---------------------------------------------------------------------------
// Rollout → JSON-RPC message conversion
// ---------------------------------------------------------------------------

function extractCommand(name, argsStr) {
  try {
    const a = JSON.parse(argsStr);
    if (a.cmd) return { command: a.cmd, cwd: a.workdir || "" };
    if (typeof a.command === "string") return { command: a.command, cwd: a.workdir || "" };
    if (Array.isArray(a.command)) return { command: a.command.join(" "), cwd: a.workdir || "" };
  } catch {
    // ignore
  }
  return { command: argsStr, cwd: "" };
}

function extractFilePaths(patchText) {
  const paths = [];
  const re = /\*\*\* (?:Update|Add|Delete) File:\s*(.+)/g;
  let m;
  while ((m = re.exec(patchText)) !== null) {
    paths.push(m[1].trim());
  }
  return paths;
}

function parseOutputJson(outputStr) {
  try {
    return JSON.parse(outputStr);
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------------------
// Per-turn evidence collection & LLM summarization
// ---------------------------------------------------------------------------

/** Collects commands, file paths, errors, and agent messages per turn. */
class TurnEvidenceCollector {
  constructor() {
    /** @type {Map<string, {commands: string[], filePaths: string[], errors: string[], agentMessage: string|null, status: string, parentTurnId: string|null}>} */
    this.byTurn = new Map();
  }

  _ensure(turnId) {
    if (!this.byTurn.has(turnId)) {
      this.byTurn.set(turnId, {
        commands: [],
        filePaths: [],
        errors: [],
        agentMessage: null,
        status: "completed",
        parentTurnId: null,
      });
    }
    return this.byTurn.get(turnId);
  }

  addCommand(turnId, cmd, exitCode) {
    const t = this._ensure(turnId);
    t.commands.push(cmd);
    if (exitCode !== 0 && exitCode !== null) {
      t.errors.push(`${cmd} (exit ${exitCode})`);
    }
  }

  addFilePath(turnId, path) {
    const t = this._ensure(turnId);
    if (!t.filePaths.includes(path)) t.filePaths.push(path);
  }

  setAgentMessage(turnId, msg) {
    const t = this._ensure(turnId);
    t.agentMessage = msg;
  }

  setStatus(turnId, status) {
    const t = this._ensure(turnId);
    t.status = status;
  }

  setParentTurnId(turnId, parentTurnId) {
    const t = this._ensure(turnId);
    t.parentTurnId = parentTurnId;
  }
}

function computeSignal(evidence) {
  if (evidence.errors.length > 0) return "error";
  if (evidence.filePaths.length > 0) return "code_changes";
  if (evidence.commands.length > 0) return "execution";
  return "status_only";
}

function buildCompactSummary(turnId, evidence) {
  const { commands, filePaths, errors, agentMessage, status } = evidence;
  const parts = [];

  // Lead with action verb + specifics
  if (errors.length > 0) {
    const cmdExamples = commands.slice(0, 2).join(", ");
    parts.push(`Failed ${errors.length} command${errors.length !== 1 ? "s" : ""}${cmdExamples ? ` (${cmdExamples})` : ""}`);
  } else if (filePaths.length > 0 && commands.length > 0) {
    const fileExamples = filePaths.slice(0, 2).map(f => f.split("/").pop()).join(", ");
    const cmdExamples = commands.slice(0, 2).join(", ");
    parts.push(`Edited ${filePaths.length} file${filePaths.length !== 1 ? "s" : ""} (${fileExamples}), ran ${commands.length} command${commands.length !== 1 ? "s" : ""} (${cmdExamples})`);
  } else if (filePaths.length > 0) {
    const fileExamples = filePaths.slice(0, 3).map(f => f.split("/").pop()).join(", ");
    parts.push(`Edited ${filePaths.length} file${filePaths.length !== 1 ? "s" : ""} (${fileExamples})`);
  } else if (commands.length > 0) {
    const cmdExamples = commands.slice(0, 3).join(", ");
    parts.push(`Ran ${commands.length} command${commands.length !== 1 ? "s" : ""} (${cmdExamples})`);
  } else {
    parts.push(status === "completed" ? "Completed" : `Status: ${status}`);
  }

  // Add agent message snippet
  if (agentMessage) {
    const snippet = agentMessage.replace(/\s+/g, " ").trim().slice(0, 100);
    parts.push(snippet);
  }

  return parts.join(". ").slice(0, MAX_SYNTHETIC_SUMMARY_CHARS);
}

function buildEvidenceBasedSummaryNode(threadId, turnId, evidence) {
  const signal = computeSignal(evidence);
  const compactSummary = buildCompactSummary(turnId, evidence);
  const agentSnippet = evidence.agentMessage
    ? evidence.agentMessage.replace(/\s+/g, " ").trim().slice(0, 120)
    : null;

  return {
    node_id: `turn:${turnId}`,
    parent_id: evidence.parentTurnId ? `turn:${evidence.parentTurnId}` : null,
    node_type: "turn",
    title: `Turn ${turnId}`,
    summary: compactSummary,
    status: evidence.status,
    brief: {
      signal,
      agent_message: agentSnippet,
      primary_command: evidence.commands[0] ?? null,
      primary_file_path: evidence.filePaths[0] ?? null,
      primary_error: evidence.errors[0] ?? null,
    },
    lineage: {
      parent_turn_id: evidence.parentTurnId ?? null,
      forked_from_thread_id: null,
      started_after_rollback: false,
    },
    counts: {
      commands_total: evidence.commands.length,
      commands_indexed: Math.min(evidence.commands.length, 5),
      commands_omitted: Math.max(0, evidence.commands.length - 5),
      file_paths_total: evidence.filePaths.length,
      file_paths_indexed: Math.min(evidence.filePaths.length, 5),
      file_paths_omitted: Math.max(0, evidence.filePaths.length - 5),
      errors_total: evidence.errors.length,
      errors_indexed: Math.min(evidence.errors.length, 5),
      errors_omitted: Math.max(0, evidence.errors.length - 5),
    },
    digest: {
      command_examples: evidence.commands.slice(0, 5),
      file_path_examples: evidence.filePaths.slice(0, 5),
      error_examples: evidence.errors.slice(0, 5),
    },
    evidence: {
      file_paths: evidence.filePaths.slice(0, 10),
      commands: evidence.commands.slice(0, 10).map((cmd) => ({
        command: cmd,
        exit_code: evidence.errors.some((e) => e.startsWith(cmd)) ? 1 : 0,
      })),
      errors: evidence.errors.slice(0, 10),
    },
  };
}

/**
 * Call OpenAI to generate LLM-enhanced summaries for turns missing persisted ones.
 * Falls back gracefully if OPENAI_API_KEY is not set.
 */
async function generateLLMSummaries(turnEvidence) {
  const apiKey = process.env.OPENAI_API_KEY;
  if (!apiKey) {
    console.log("[summary] No OPENAI_API_KEY set — using mechanical summaries only");
    return new Map();
  }

  const inputs = [];
  for (const [turnId, ev] of turnEvidence) {
    inputs.push({
      turn_id: turnId,
      status: ev.status,
      parent_turn_id: ev.parentTurnId,
      signal: computeSignal(ev),
      last_agent_message: ev.agentMessage?.slice(0, 200) ?? null,
      primary_command: ev.commands[0] ?? null,
      primary_file_path: ev.filePaths[0] ?? null,
      primary_error: ev.errors[0] ?? null,
      total_commands: ev.commands.length,
      total_file_paths: ev.filePaths.length,
      total_errors: ev.errors.length,
      command_examples: ev.commands.slice(0, 3),
      file_path_examples: ev.filePaths.slice(0, 3),
      error_examples: ev.errors.slice(0, 3),
    });
  }

  if (inputs.length === 0) return new Map();

  console.log(`[summary] Calling ${SUMMARY_LLM_MODEL} for ${inputs.length} turn(s)…`);
  try {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), SUMMARY_LLM_TIMEOUT_MS);

    const res = await fetch("https://api.openai.com/v1/chat/completions", {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${apiKey}`,
      },
      body: JSON.stringify({
        model: SUMMARY_LLM_MODEL,
        max_completion_tokens: SUMMARY_LLM_MAX_TOKENS,
        temperature: 0.3,
        messages: [
          { role: "system", content: SUMMARY_SYSTEM_PROMPT },
          { role: "user", content: JSON.stringify(inputs, null, 2) },
        ],
      }),
      signal: controller.signal,
    });
    clearTimeout(timeout);

    if (!res.ok) {
      const text = await res.text().catch(() => "");
      console.error(`[summary] OpenAI API error ${res.status}: ${text.slice(0, 200)}`);
      return new Map();
    }

    const data = await res.json();
    const raw = data.choices?.[0]?.message?.content ?? "[]";
    // Parse response — handle markdown fences
    let items;
    try {
      items = JSON.parse(raw);
    } catch {
      const start = raw.indexOf("[");
      const end = raw.lastIndexOf("]");
      if (start >= 0 && end > start) {
        items = JSON.parse(raw.slice(start, end + 1));
      } else {
        throw new Error("no JSON array found");
      }
    }

    const result = new Map();
    for (const item of items) {
      if (item.turn_id && (item.short_summary || item.detail_summary || item.summary)) {
        result.set(item.turn_id, {
          short: item.short_summary || item.summary || "",
          detail: item.detail_summary || item.summary || "",
        });
      }
    }
    console.log(`[summary] Got LLM summaries for ${result.size} turn(s)`);
    return result;
  } catch (err) {
    console.error(`[summary] LLM call failed: ${err.message}`);
    return new Map();
  }
}

function isStructuredSummary(text) {
  return typeof text === "string" && text.trim().length > 0;
}

function buildReplayMessages(rolloutLines) {
  const messages = [];
  let threadId = null;
  let currentTurnId = null;
  let turnCounter = 0;
  const completedTurnIds = [];
  const hasTaskStarted = rolloutLines.some(
    (l) => l.type === "event_msg" && l.payload?.type === "task_started",
  );
  const pendingCalls = new Map();
  const summaryFallbackByTurn = new Map();
  const turnParentMap = new Map();
  const turnCompletionCache = new Map();
  const evidenceCollector = new TurnEvidenceCollector();

  function ensureTurnOpen(ts) {
    if (!currentTurnId) {
      turnCounter++;
      currentTurnId = `turn-${turnCounter}`;
      messages.push({
        ts,
        msg: {
          method: "turn/started",
          params: {
            threadId: threadId || "",
            turn: { id: currentTurnId, items: [], status: "inProgress", error: null },
          },
        },
      });
    }
  }

  for (const line of rolloutLines) {
    const ts = line.timestamp;
    const t = line.type;
    const p = line.payload;

    if (t === "session_meta") {
      threadId = p.id;
      const now = Math.floor(Date.now() / 1000);
      messages.push({
        ts,
        msg: {
          method: "thread/started",
          params: {
            thread: {
              id: threadId,
              preview: "",
              ephemeral: false,
              modelProvider: p.model_provider || "openai",
              createdAt: now,
              updatedAt: now,
              status: "inProgress",
              path: null,
              cwd: p.cwd || "",
              cliVersion: p.cli_version || "",
              source: p.source || "cli",
              agentNickname: null,
              agentRole: null,
              gitInfo: p.git
                ? {
                    commitHash: p.git.commit_hash || "",
                    branch: p.git.branch || "",
                    repositoryUrl: p.git.repository_url || "",
                  }
                : null,
              name: null,
              turns: [],
            },
          },
        },
      });
      continue;
    }

    if (t === "event_msg") {
      const sub = p.type;

      if (sub === "task_started") {
        if (currentTurnId) {
          turnParentMap.set(p.turn_id, currentTurnId);
          evidenceCollector.setParentTurnId(p.turn_id, currentTurnId);
        }
        currentTurnId = p.turn_id;
        messages.push({
          ts,
          msg: {
            method: "turn/started",
            params: {
              threadId: threadId || "",
              turn: { id: currentTurnId, items: [], status: "inProgress", error: null },
            },
          },
        });
        continue;
      }

      if (sub === "user_message") {
        ensureTurnOpen(ts);
        messages.push({
          ts,
          msg: {
            method: "item/completed",
            params: {
              threadId: threadId || "",
              turnId: currentTurnId || "",
              item: {
                type: "userMessage",
                id: `user-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
                content: [{ type: "text", text: p.message || "" }],
              },
            },
          },
        });
        continue;
      }

      if (sub === "task_complete") {
        const completedTurnId = p.turn_id || currentTurnId || "";
        const summaryText = p.last_agent_message ?? "";
        const parentTurnId = turnParentMap.get(completedTurnId) ?? null;
        // Record agent message and status in evidence
        if (completedTurnId) {
          evidenceCollector.setStatus(completedTurnId, "completed");
          if (summaryText) evidenceCollector.setAgentMessage(completedTurnId, summaryText);
          if (parentTurnId) evidenceCollector.setParentTurnId(completedTurnId, parentTurnId);
        }
        // Try persisted summary first, then build from evidence
        const persistedNode = readTurnSummaryNode(threadId, completedTurnId);
        const summaryNode = persistedNode
          ?? (completedTurnId && evidenceCollector.byTurn.has(completedTurnId)
            ? buildEvidenceBasedSummaryNode(threadId ?? "", completedTurnId, evidenceCollector.byTurn.get(completedTurnId))
            : resolveSummaryNode(threadId, completedTurnId, "completed", parentTurnId, summaryText, summaryFallbackByTurn));
        summaryFallbackByTurn.set(completedTurnId, summaryNode);
        if (summaryNode) {
          messages.push({
            ts,
            msg: {
              method: "agentcanvas/summaryNode",
              params: {
                threadId: threadId || "",
                turnId: completedTurnId,
                node: summaryNode,
              },
            },
          });
        }
        messages.push({
          ts,
          msg: {
            method: "turn/completed",
            params: {
              threadId: threadId || "",
              turn: {
                id: completedTurnId,
                items: [],
                status: "completed",
                error: null,
              },
            },
          },
        });
        if (completedTurnId) completedTurnIds.push(completedTurnId);
        currentTurnId = null;
        continue;
      }

      if (sub === "turn_aborted") {
        const completedTurnId = p.turn_id || currentTurnId || "";
        const summaryText = p.last_agent_message ?? "";
        const parentTurnId = turnParentMap.get(completedTurnId) ?? null;
        if (completedTurnId) {
          evidenceCollector.setStatus(completedTurnId, "interrupted");
          if (summaryText) evidenceCollector.setAgentMessage(completedTurnId, summaryText);
          if (parentTurnId) evidenceCollector.setParentTurnId(completedTurnId, parentTurnId);
        }
        const persistedNode = readTurnSummaryNode(threadId, completedTurnId);
        const summaryNode = persistedNode
          ?? (completedTurnId && evidenceCollector.byTurn.has(completedTurnId)
            ? buildEvidenceBasedSummaryNode(threadId ?? "", completedTurnId, evidenceCollector.byTurn.get(completedTurnId))
            : resolveSummaryNode(threadId, completedTurnId, "interrupted", parentTurnId, summaryText, summaryFallbackByTurn));
        summaryFallbackByTurn.set(completedTurnId, summaryNode);
        if (summaryNode) {
          messages.push({
            ts,
            msg: {
              method: "agentcanvas/summaryNode",
              params: {
                threadId: threadId || "",
                turnId: completedTurnId,
                node: summaryNode,
              },
            },
          });
        }
        messages.push({
          ts,
          msg: {
            method: "turn/completed",
            params: {
              threadId: threadId || "",
              turn: {
                id: completedTurnId,
                items: [],
                status: "interrupted",
                error: null,
              },
            },
          },
        });
        if (completedTurnId) completedTurnIds.push(completedTurnId);
        currentTurnId = null;
        continue;
      }

      if (sub === "thread_rolled_back") {
        const numTurns = Number(p.num_turns ?? p.numTurns ?? 0);
        if (numTurns > 0) {
          const start = Math.max(0, completedTurnIds.length - numTurns);
          const rolledBackTurnIds = completedTurnIds.splice(start, numTurns);
          for (const rolledBackTurnId of rolledBackTurnIds) {
            const rollbackFallbackEnvelope = makeFallbackSummaryNode(
                threadId,
                rolledBackTurnId,
                "rolled_back",
                turnParentMap.get(rolledBackTurnId) ?? null,
                "",
              );
            const summaryNode = turnCompletionCache.get(rolledBackTurnId)
              ?? readTurnSummaryNode(threadId, rolledBackTurnId)
              ?? summaryFallbackByTurn.get(rolledBackTurnId)
              ?? rollbackFallbackEnvelope?.nodes?.[0] ?? rollbackFallbackEnvelope;
            if (summaryNode) turnCompletionCache.set(rolledBackTurnId, summaryNode);
            if (!summaryNode) continue;
            messages.push({
              ts,
              msg: {
                method: "agentcanvas/summaryNode",
                params: {
                  threadId: threadId || "",
                  turnId: rolledBackTurnId,
                  node: summaryNode,
                },
              },
            });
          }
        }
        continue;
      }

      continue;
    }

    if (t === "response_item") {
      const sub = p.type;

      if (sub === "function_call") {
        pendingCalls.set(p.call_id, { name: p.name, arguments: p.arguments || "", timestamp: ts });
        continue;
      }

      if (sub === "function_call_output") {
        const call = pendingCalls.get(p.call_id);
        pendingCalls.delete(p.call_id);
        if (!call) continue;

        ensureTurnOpen(ts);
        const name = call.name;
        const output = p.output || "";

        let exitCode = 0;
        const exitMatch = output.match(/Process exited with code (\d+)/);
        if (exitMatch) exitCode = Number(exitMatch[1]);

        if (name === "shell_command" || name === "exec_command" || name === "shell" || name === "write_stdin") {
          const { command, cwd } = extractCommand(name, call.arguments);
          if (currentTurnId) evidenceCollector.addCommand(currentTurnId, command, exitCode);
          messages.push({
            ts,
            msg: {
              method: "item/completed",
              params: {
                threadId: threadId || "",
                turnId: currentTurnId || "",
                item: {
                  type: "commandExecution",
                  id: p.call_id,
                  command,
                  cwd: cwd || "",
                  processId: null,
                  status: exitCode === 0 ? "completed" : "failed",
                  commandActions: [],
                  aggregatedOutput: output,
                  exitCode,
                  durationMs: null,
                },
              },
            },
          });
          continue;
        }

        if (name === "apply_patch") {
          const filePaths = extractFilePaths(call.arguments);
          if (currentTurnId) {
            for (const fp of filePaths) evidenceCollector.addFilePath(currentTurnId, fp);
          }
          messages.push({
            ts,
            msg: {
              method: "item/completed",
              params: {
                threadId: threadId || "",
                turnId: currentTurnId || "",
                item: {
                  type: "fileChange",
                  id: p.call_id,
                  changes: filePaths.map((fp) => ({
                    path: fp,
                    kind: { type: "update", move_path: null },
                    diff: call.arguments,
                  })),
                  status: "completed",
                },
              },
            },
          });
          continue;
        }

        let parsedArgs = {};
        try { parsedArgs = JSON.parse(call.arguments); } catch { /* keep empty */ }
        messages.push({
          ts,
          msg: {
            method: "item/completed",
            params: {
              threadId: threadId || "",
              turnId: currentTurnId || "",
              item: {
                type: "mcpToolCall",
                id: p.call_id,
                server: "unknown",
                tool: name,
                status: "completed",
                arguments: parsedArgs,
                result: { type: "text", text: output },
                error: null,
                durationMs: null,
              },
            },
          },
        });
        continue;
      }

      if (sub === "custom_tool_call") {
        pendingCalls.set(p.call_id, { name: p.name, input: p.input || "", timestamp: ts });
        continue;
      }

      if (sub === "custom_tool_call_output") {
        const call = pendingCalls.get(p.call_id);
        pendingCalls.delete(p.call_id);
        ensureTurnOpen(ts);
        if (!call) continue;

        const name = call.name;
        const rawOutput = p.output || "";
        const parsedOutput = parseOutputJson(rawOutput);

        if (name === "apply_patch") {
          const filePaths = extractFilePaths(call.input);
          if (currentTurnId) {
            for (const fp of filePaths) evidenceCollector.addFilePath(currentTurnId, fp);
          }
          messages.push({
            ts,
            msg: {
              method: "item/completed",
              params: {
                threadId: threadId || "",
                turnId: currentTurnId || "",
                item: {
                  type: "fileChange",
                  id: p.call_id,
                  changes: filePaths.map((fp) => ({
                    path: fp,
                    kind: { type: "update", move_path: null },
                    diff: call.input,
                  })),
                  status: parsedOutput?.metadata?.exit_code === 0 ? "completed" : "failed",
                },
              },
            },
          });
          continue;
        }

        let parsedArgs = {};
        try { parsedArgs = typeof call.input === "string" ? { input: call.input } : call.input; } catch { /* keep empty */ }
        messages.push({
          ts,
          msg: {
            method: "item/completed",
            params: {
              threadId: threadId || "",
              turnId: currentTurnId || "",
              item: {
                type: "mcpToolCall",
                id: p.call_id,
                server: "custom",
                tool: name,
                status: parsedOutput?.metadata?.exit_code === 0 ? "completed" : "failed",
                arguments: parsedArgs,
                result: { type: "text", text: parsedOutput?.output || rawOutput },
                error: null,
                durationMs: null,
              },
            },
          },
        });
        continue;
      }

      continue;
    }

    if (t === "turn_context" && !hasTaskStarted) {
      if (currentTurnId) {
        messages.push({
          ts,
          msg: {
            method: "turn/completed",
            params: {
              threadId: threadId || "",
              turn: { id: currentTurnId, items: [], status: "completed", error: null },
            },
          },
        });
      }
      turnCounter++;
      currentTurnId = `turn-${turnCounter}`;
      messages.push({
        ts,
        msg: {
          method: "turn/started",
          params: {
            threadId: threadId || "",
            turn: { id: currentTurnId, items: [], status: "inProgress", error: null },
          },
        },
      });
      continue;
    }
  }

  if (!hasTaskStarted && currentTurnId) {
    const lastTs = rolloutLines[rolloutLines.length - 1]?.timestamp;
    messages.push({
      ts: lastTs,
      msg: {
        method: "turn/completed",
        params: {
          threadId: threadId || "",
          turn: { id: currentTurnId, items: [], status: "completed", error: null },
        },
      },
    });
  }

  return { messages, evidenceCollector, threadId };
}

function loadAndBuild(filePath) {
  const rawLines = readFileSync(filePath, "utf-8").split("\n").filter((l) => l.trim());
  const rolloutLines = rawLines.map((l) => JSON.parse(l));
  return buildReplayMessages(rolloutLines);
}

/**
 * Enhance summary nodes in messages with LLM-generated summaries.
 * Only enhances turns that don't already have a persisted summary.
 */
async function enhanceWithLLM(result) {
  const { messages, evidenceCollector, threadId } = result;
  // Identify turns that need LLM enhancement (no persisted summary)
  const turnsNeedingLLM = new Map();
  for (const [turnId, ev] of evidenceCollector.byTurn) {
    const persisted = readTurnSummaryNode(threadId, turnId);
    if (!persisted) {
      turnsNeedingLLM.set(turnId, ev);
    }
  }

  if (turnsNeedingLLM.size === 0) {
    console.log("[summary] All turns have persisted summaries");
    return messages;
  }

  const llmSummaries = await generateLLMSummaries(turnsNeedingLLM);

  // Replace summary nodes in messages with LLM-enhanced versions
  if (llmSummaries.size > 0) {
    for (const msg of messages) {
      const m = msg.msg;
      if (m.method !== "agentcanvas/summaryNode") continue;
      const turnId = m.params?.turnId;
      const llmResult = llmSummaries.get(turnId);
      if (!llmResult) continue;

      const node = m.params.node;
      if (node) {
        // Use detail_summary as the main summary text
        if (isStructuredSummary(llmResult.detail)) {
          node.summary = llmResult.detail;
        }
        // Embed short_summary into brief for graph label extraction
        if (isStructuredSummary(llmResult.short)) {
          if (!node.brief) node.brief = {};
          node.brief.short_summary = llmResult.short;
        }
      }
    }
  }

  return messages;
}

// Pre-load default if specified
let defaultResult = null;
let defaultMessages = null;
if (defaultRolloutPath) {
  defaultResult = loadAndBuild(defaultRolloutPath);
  // Enhance with LLM asynchronously
  enhanceWithLLM(defaultResult).then((msgs) => {
    defaultMessages = msgs;
    console.log(`Pre-loaded ${defaultMessages.length} messages from default rollout`);
  });
}

// ---------------------------------------------------------------------------
// HTTP + WebSocket server
// ---------------------------------------------------------------------------

const server = createServer((req, res) => {
  // CORS headers
  res.setHeader("Access-Control-Allow-Origin", "*");
  res.setHeader("Access-Control-Allow-Methods", "GET, OPTIONS");
  res.setHeader("Access-Control-Allow-Headers", "Content-Type");

  if (req.method === "OPTIONS") {
    res.writeHead(204);
    res.end();
    return;
  }

  if (req.method === "GET" && req.url === "/api/sessions") {
    const sessions = getSessionList();
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(JSON.stringify(sessions));
    return;
  }

  res.writeHead(404);
  res.end("Not found");
});

const wss = new WebSocketServer({ server });

wss.on("connection", async (ws, req) => {
  // Parse ?file= query param from the upgrade request URL
  let replayMessages = defaultMessages;
  try {
    const url = new URL(req.url, `http://localhost:${PORT}`);
    const fileParam = url.searchParams.get("file");
    if (fileParam) {
      const filePath = resolve(fileParam);
      console.log(`\n[ws] Client requested session: ${filePath}`);
      const result = loadAndBuild(filePath);
      replayMessages = await enhanceWithLLM(result);
    }
  } catch (e) {
    console.error("[ws] Error parsing connection URL:", e.message);
  }

  if (!replayMessages || replayMessages.length === 0) {
    console.log("[ws] No replay messages available");
    ws.close();
    return;
  }

  console.log(`[ws] Client connected — replaying ${replayMessages.length} messages...`);

  let prevTs = null;

  for (let i = 0; i < replayMessages.length; i++) {
    if (ws.readyState !== ws.OPEN) {
      console.log("[ws] Client disconnected, stopping replay");
      return;
    }

    const { ts, msg } = replayMessages[i];

    if (prevTs) {
      const delta = (new Date(ts) - new Date(prevTs)) / SPEED;
      const capped = Math.min(delta, 2000);
      if (capped > 10) {
        await sleep(capped);
      }
    }
    prevTs = ts;

    ws.send(JSON.stringify(msg));

    const itemType = msg.params?.item?.type || "";
    const detail = itemType ? ` (${itemType})` : "";
    console.log(`  [${i + 1}/${replayMessages.length}] ${msg.method}${detail}`);
  }

  console.log("\n[ws] Replay complete! Connection kept open.");
});

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

server.listen(PORT, "127.0.0.1", () => {
  console.log(`\nReplay server running on http://localhost:${PORT}`);
  console.log(`  GET /api/sessions — list available sessions`);
  console.log(`  WS  ws://localhost:${PORT}[?file=<path>] — replay a session`);
  console.log(`  Speed: ${SPEED}x`);
});
