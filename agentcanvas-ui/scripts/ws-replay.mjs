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

import { readFileSync, readdirSync } from "node:fs";
import { createServer } from "node:http";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { URL } from "node:url";
import { WebSocketServer } from "ws";

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

  return makeFallbackSummaryNode(
    threadIdValue,
    turnIdValue,
    status,
    parentTurnId,
    lastAgentMessage,
  );
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

function pad2(n) {
  return String(n).padStart(2, "0");
}

function formatLocalDateTime(tsValue) {
  const ts = new Date(tsValue);
  if (Number.isNaN(ts.getTime())) {
    return { date: "1970-01-01", time: "00:00" };
  }
  return {
    date: `${ts.getFullYear()}-${pad2(ts.getMonth() + 1)}-${pad2(ts.getDate())}`,
    time: `${pad2(ts.getHours())}:${pad2(ts.getMinutes())}`,
  };
}

function extractMessageText(content) {
  if (!Array.isArray(content)) return "";
  for (const part of content) {
    if (!part || typeof part !== "object") continue;
    if (typeof part.text === "string" && part.text.trim()) return part.text;
  }
  return "";
}

function isSetupMessage(text) {
  const trimmed = text.trim();
  return (
    trimmed.startsWith("<user_instructions>") ||
    trimmed.startsWith("<environment_context>") ||
    trimmed.startsWith("<turn_aborted>") ||
    trimmed.startsWith("# AGENTS.md instructions") ||
    trimmed.startsWith("<permissions instructions>")
  );
}

function extractCwdFromText(text) {
  const m = text.match(/<cwd>([^<]+)<\/cwd>/);
  return m?.[1]?.trim() || "";
}

function parseArgumentsObject(args) {
  if (typeof args === "object" && args !== null) return args;
  if (typeof args !== "string") return null;
  try {
    return JSON.parse(args);
  } catch {
    return null;
  }
}

function readSessionInfo(filePath) {
  try {
    const content = readFileSync(filePath, "utf-8");
    const lines = content.split("\n").filter((l) => l.trim());
    const parsed = [];
    for (const line of lines) {
      try {
        parsed.push(JSON.parse(line));
      } catch {
        // skip malformed line
      }
    }
    if (parsed.length === 0) return null;

    let meta = null;
    let taskStartedCount = 0;
    let turnContextCount = 0;

    for (const obj of parsed) {
      if (obj.type === "session_meta" && !meta) meta = obj.payload;
      if (obj.type === "event_msg" && obj.payload?.type === "task_started") taskStartedCount++;
      if (obj.type === "turn_context") turnContextCount++;
    }

    if (meta) {
      const turns = taskStartedCount > 0 ? taskStartedCount : turnContextCount;
      const { date, time } = formatLocalDateTime(meta.timestamp);
      return {
        date,
        time,
        turns,
        source: meta.source || "unknown",
        cwd: meta.cwd || "",
        file: filePath,
        id: meta.id,
      };
    }

    const header = parsed[0];
    if (!header?.id || !header?.timestamp) return null;

    let turns = 0;
    let cwd = "";
    for (const obj of parsed) {
      if (obj.type === "message" && obj.role === "user") {
        const text = extractMessageText(obj.content);
        if (text && !isSetupMessage(text)) turns++;
        if (!cwd && text) cwd = extractCwdFromText(text);
      }
      if (!cwd && obj.type === "function_call") {
        const args = parseArgumentsObject(obj.arguments);
        const argCwd = typeof args?.workdir === "string" ? args.workdir : "";
        if (argCwd) cwd = argCwd;
      }
    }

    const { date, time } = formatLocalDateTime(header.timestamp);
    return {
      date,
      time,
      turns,
      source: "cli",
      cwd,
      file: filePath,
      id: header.id,
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

function extractOutputText(rawOutput) {
  const parsedOutput = parseOutputJson(rawOutput);
  if (typeof parsedOutput?.output === "string") return parsedOutput.output;
  return rawOutput;
}

function extractExitCode(rawOutput) {
  const parsedOutput = parseOutputJson(rawOutput);
  if (typeof parsedOutput?.metadata?.exit_code === "number") {
    return parsedOutput.metadata.exit_code;
  }
  const exitMatch = rawOutput.match(/Process exited with code (\d+)/);
  if (exitMatch) return Number(exitMatch[1]);
  return 0;
}

function buildReplayMessagesLegacy(rolloutLines) {
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
        if (currentTurnId) turnParentMap.set(p.turn_id, currentTurnId);
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
        const summaryNode = resolveSummaryNode(
          threadId,
          completedTurnId,
          "completed",
          parentTurnId,
          summaryText,
          summaryFallbackByTurn,
        );
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
        const summaryNode = resolveSummaryNode(
          threadId,
          completedTurnId,
          "interrupted",
          parentTurnId,
          summaryText,
          summaryFallbackByTurn,
        );
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
            const summaryNode = turnCompletionCache.get(rolledBackTurnId)
              ?? readTurnSummaryNode(threadId, rolledBackTurnId)
              ?? summaryFallbackByTurn.get(rolledBackTurnId)
              ?? makeFallbackSummaryNode(
                threadId,
                rolledBackTurnId,
                "rolled_back",
                turnParentMap.get(rolledBackTurnId) ?? null,
                "",
              );
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
        const rawOutput = p.output || "";
        const output = extractOutputText(rawOutput);
        const exitCode = extractExitCode(rawOutput);

        if (name === "shell_command" || name === "exec_command" || name === "shell" || name === "write_stdin") {
          const { command, cwd } = extractCommand(name, call.arguments);
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

  return messages;
}

function buildReplayMessagesModern(rolloutLines) {
  const messages = [];
  const pendingCalls = new Map();
  const header = rolloutLines[0] || {};
  const threadId = header.id || `thread-${Date.now()}`;
  const startTs = header.timestamp || new Date().toISOString();
  let currentTurnId = null;
  let turnCounter = 0;

  let cwd = typeof header.cwd === "string" ? header.cwd : "";
  for (const line of rolloutLines) {
    if (cwd) break;
    if (line.type === "message" && line.role === "user") {
      const text = extractMessageText(line.content);
      if (text) cwd = extractCwdFromText(text);
    }
    if (!cwd && line.type === "function_call") {
      const args = parseArgumentsObject(line.arguments);
      if (typeof args?.workdir === "string") cwd = args.workdir;
    }
  }

  const now = Math.floor(Date.now() / 1000);
  messages.push({
    ts: startTs,
    msg: {
      method: "thread/started",
      params: {
        thread: {
          id: threadId,
          preview: "",
          ephemeral: false,
          modelProvider: header.model_provider || "openai",
          createdAt: now,
          updatedAt: now,
          status: "inProgress",
          path: null,
          cwd,
          cliVersion: header.cli_version || "",
          source: header.source || "cli",
          agentNickname: null,
          agentRole: null,
          gitInfo: header.git
            ? {
                commitHash: header.git.commit_hash || "",
                branch: header.git.branch || "",
                repositoryUrl: header.git.repository_url || "",
              }
            : null,
          name: null,
          turns: [],
        },
      },
    },
  });

  function startTurn(ts, userText = null) {
    turnCounter++;
    currentTurnId = `turn-${turnCounter}`;
    messages.push({
      ts,
      msg: {
        method: "turn/started",
        params: {
          threadId,
          turn: { id: currentTurnId, items: [], status: "inProgress", error: null },
        },
      },
    });
    if (userText) {
      messages.push({
        ts,
        msg: {
          method: "item/completed",
          params: {
            threadId,
            turnId: currentTurnId,
            item: {
              type: "userMessage",
              id: `user-${turnCounter}`,
              content: [{ type: "text", text: userText }],
            },
          },
        },
      });
    }
  }

  function closeTurn(ts, status = "completed") {
    if (!currentTurnId) return;
    messages.push({
      ts,
      msg: {
        method: "turn/completed",
        params: {
          threadId,
          turn: { id: currentTurnId, items: [], status, error: null },
        },
      },
    });
    currentTurnId = null;
  }

  function ensureTurnOpen(ts) {
    if (!currentTurnId) startTurn(ts);
  }

  for (const line of rolloutLines.slice(1)) {
    const ts = line.timestamp || startTs;

    if (line.type === "message" && line.role === "user") {
      const text = extractMessageText(line.content);
      if (!text || isSetupMessage(text)) continue;
      if (currentTurnId) closeTurn(ts);
      startTurn(ts, text);
      continue;
    }

    if (line.type === "function_call") {
      pendingCalls.set(line.call_id, { name: line.name || "", arguments: line.arguments || "" });
      ensureTurnOpen(ts);
      continue;
    }

    if (line.type === "function_call_output") {
      const call = pendingCalls.get(line.call_id);
      pendingCalls.delete(line.call_id);
      if (!call) continue;

      ensureTurnOpen(ts);
      const rawOutput = line.output || "";
      const output = extractOutputText(rawOutput);
      const exitCode = extractExitCode(rawOutput);
      const callArgs = typeof call.arguments === "string" ? call.arguments : JSON.stringify(call.arguments);

      if (call.name === "shell_command" || call.name === "exec_command" || call.name === "shell" || call.name === "write_stdin") {
        const { command, cwd: cmdCwd } = extractCommand(call.name, callArgs);
        messages.push({
          ts,
          msg: {
            method: "item/completed",
            params: {
              threadId,
              turnId: currentTurnId,
              item: {
                type: "commandExecution",
                id: line.call_id,
                command,
                cwd: cmdCwd || "",
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

      if (call.name === "apply_patch") {
        const filePaths = extractFilePaths(callArgs);
        messages.push({
          ts,
          msg: {
            method: "item/completed",
            params: {
              threadId,
              turnId: currentTurnId,
              item: {
                type: "fileChange",
                id: line.call_id,
                changes: filePaths.map((fp) => ({
                  path: fp,
                  kind: { type: "update", move_path: null },
                  diff: callArgs,
                })),
                status: exitCode === 0 ? "completed" : "failed",
              },
            },
          },
        });
        continue;
      }

      let parsedArgs = {};
      try {
        parsedArgs = JSON.parse(callArgs);
      } catch {
        parsedArgs = { input: callArgs };
      }

      messages.push({
        ts,
        msg: {
          method: "item/completed",
          params: {
            threadId,
            turnId: currentTurnId,
            item: {
              type: "mcpToolCall",
              id: line.call_id,
              server: "unknown",
              tool: call.name,
              status: exitCode === 0 ? "completed" : "failed",
              arguments: parsedArgs,
              result: { type: "text", text: output },
              error: null,
              durationMs: null,
            },
          },
        },
      });
    }
  }

  const lastTs = rolloutLines[rolloutLines.length - 1]?.timestamp || startTs;
  closeTurn(lastTs);

  return messages;
}

function buildReplayMessages(rolloutLines) {
  const isLegacy = rolloutLines.some(
    (l) => l.type === "session_meta" || l.type === "event_msg" || l.type === "response_item" || l.type === "turn_context",
  );
  return isLegacy ? buildReplayMessagesLegacy(rolloutLines) : buildReplayMessagesModern(rolloutLines);
}

function loadAndBuild(filePath) {
  const rawLines = readFileSync(filePath, "utf-8").split("\n").filter((l) => l.trim());
  const rolloutLines = rawLines.map((l) => JSON.parse(l));
  return buildReplayMessages(rolloutLines);
}

// Pre-load default if specified
let defaultMessages = null;
if (defaultRolloutPath) {
  defaultMessages = loadAndBuild(defaultRolloutPath);
  console.log(`Pre-loaded ${defaultMessages.length} messages from default rollout`);
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
      replayMessages = loadAndBuild(filePath);
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
