#!/usr/bin/env node

/**
 * ws-replay.mjs — Replay historical Codex rollout sessions over WebSocket.
 *
 * Supports:
 *   - One-shot replay from a rollout file
 *   - Live mode with --watch to tail explicit file or latest session
 */

import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { createServer } from "node:http";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { URL } from "node:url";
import { WebSocketServer } from "ws";

const WATCH_POLL_INTERVAL_MS = 1000;
const MAX_SUMMARY_BODY_BYTES = 128 * 1024;
const SUMMARY_OPENAI_SYSTEM_PROMPT = `
You summarize one engineering turn into structured UI output.
Return strict JSON with this shape:
{
  "summaryText": "string",
  "signal": "error|agent_message|command_activity|file_change|status_only",
  "primaryCommand": "string|null",
  "primaryFilePath": "string|null",
  "primaryError": "string|null"
}
Rules:
- Keep summaryText concise (max 180 chars).
- Be factual; do not invent details.
- If there is an error, signal must be "error".
- If no useful details exist, set signal to "status_only".
`.trim();

const args = process.argv.slice(2);

const argValue = (name) => {
  const idx = args.indexOf(name);
  return idx === -1 ? undefined : args[idx + 1];
};

const hasFlag = (name) => args.includes(name);

const rawPort = Number(argValue("--port") ?? 8080);
const rawSpeed = Number(argValue("--speed") ?? 10);
const PORT = Number.isFinite(rawPort) && rawPort > 0 ? Math.floor(rawPort) : 8080;
const SPEED = Number.isFinite(rawSpeed) && rawSpeed > 0 ? rawSpeed : 10;
const WATCH_MODE = hasFlag("--watch");
const USE_LATEST = hasFlag("--latest");
const SUMMARY_MODE = (process.env.AGENTCANVAS_SUMMARY_MODE || "local").toLowerCase();
const OPENAI_API_KEY = process.env.AGENTCANVAS_OPENAI_API_KEY || process.env.OPENAI_API_KEY || "";
const OPENAI_BASE_URL = (
  process.env.AGENTCANVAS_OPENAI_BASE_URL
  || process.env.OPENAI_BASE_URL
  || "https://api.openai.com/v1"
).replace(/\/$/, "");
const OPENAI_SUMMARY_MODEL = process.env.AGENTCANVAS_SUMMARY_MODEL || "gpt-4o-mini";

let fileArg;
for (let i = 0; i < args.length; i += 1) {
  const arg = args[i];
  if (arg === "--port" || arg === "--speed") {
    i += 1;
    continue;
  }
  if (arg === "--watch" || arg === "--latest") {
    continue;
  }
  if (arg.startsWith("--")) {
    continue;
  }
  fileArg = resolve(arg);
  break;
}

const SESSIONS_DIR = join(homedir(), ".codex", "sessions");
const INITIAL_FILE = fileArg ?? null;

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function safeParse(line) {
  try {
    return JSON.parse(line);
  } catch {
    return null;
  }
}

function toEpochMs(value) {
  if (typeof value === "number") {
    return value > 1e12 ? value : value * 1000;
  }
  const parsed = Date.parse(String(value));
  return Number.isNaN(parsed) ? Date.now() : parsed;
}

function parseOutputJson(output) {
  try {
    return JSON.parse(output);
  } catch {
    return null;
  }
}

function extractCommand(rawArgs) {
  try {
    const parsed = JSON.parse(rawArgs);
    if (parsed.cmd) return { command: parsed.cmd, cwd: parsed.workdir || "" };
    if (typeof parsed.command === "string") return { command: parsed.command, cwd: parsed.workdir || "" };
    if (Array.isArray(parsed.command)) return { command: parsed.command.join(" "), cwd: parsed.workdir || "" };
  } catch {
    // keep fallback
  }
  return { command: rawArgs, cwd: "" };
}

function extractFilePaths(patchText) {
  const matcher = /\*\*\* (?:Update|Add|Delete) File:\s*(.+)/g;
  const paths = [];
  let match;
  while ((match = matcher.exec(patchText)) !== null) {
    paths.push(match[1].trim());
  }
  return paths;
}

function planStatusToken(status) {
  switch (status) {
    case "completed":
      return "[x]";
    case "in_progress":
      return "[-]";
    case "pending":
      return "[ ]";
    default:
      return "[ ]";
  }
}

function buildPlanTextFromArgs(rawArgs) {
  let parsed = null;
  if (typeof rawArgs === "string") {
    try {
      parsed = JSON.parse(rawArgs);
    } catch {
      parsed = null;
    }
  } else if (rawArgs && typeof rawArgs === "object") {
    parsed = rawArgs;
  }

  if (!parsed || typeof parsed !== "object") return "";

  const stepLines = [];
  if (Array.isArray(parsed.plan)) {
    for (const item of parsed.plan) {
      if (!item || typeof item !== "object") continue;
      const step = typeof item.step === "string" ? item.step.trim() : "";
      if (!step) continue;
      const status = typeof item.status === "string" ? item.status : "";
      stepLines.push(`${planStatusToken(status)} ${step}`);
    }
  }

  if (stepLines.length === 0) return "";

  const lines = [];
  const explanation = typeof parsed.explanation === "string" ? parsed.explanation.trim() : "";
  if (explanation) lines.push(explanation);
  lines.push(...stepLines);

  return lines.join("\n").trim();
}

function findRolloutFiles(dir) {
  const files = [];
  try {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const fullPath = join(dir, entry.name);
      if (entry.isDirectory()) {
        files.push(...findRolloutFiles(fullPath));
      } else if (entry.name.startsWith("rollout-") && entry.name.endsWith(".jsonl")) {
        files.push(fullPath);
      }
    }
  } catch {
    // directory unavailable
  }
  return files;
}

function readSessionInfo(filePath) {
  try {
    const text = readFileSync(filePath, "utf-8");
    const lines = text.split("\n").filter((line) => line.trim());

    let meta = null;
    let taskStartedCount = 0;
    let turnContextCount = 0;

    for (const line of lines) {
      const obj = safeParse(line);
      if (!obj || typeof obj !== "object") continue;

      if (obj.type === "session_meta" && !meta) {
        meta = obj.payload;
      }
      if (obj.type === "event_msg" && obj.payload?.type === "task_started") {
        taskStartedCount += 1;
      }
      if (obj.type === "turn_context") {
        turnContextCount += 1;
      }
    }

    if (!meta) return null;
    const ts = new Date(meta.timestamp);
    return {
      date: ts.toISOString().slice(0, 10),
      time: ts.toISOString().slice(11, 16),
      turns: taskStartedCount > 0 ? taskStartedCount : turnContextCount,
      source: meta.source || "unknown",
      cwd: meta.cwd || "",
      file: filePath,
      id: meta.id,
    };
  } catch {
    return null;
  }
}

function getSessionList() {
  return findRolloutFiles(SESSIONS_DIR)
    .sort()
    .reverse()
    .map((file) => readSessionInfo(file))
    .filter(Boolean);
}

function getLatestSessionFile() {
  const sessions = findRolloutFiles(SESSIONS_DIR).sort();
  return sessions.length === 0 ? null : sessions.at(-1) ?? null;
}

function createReplayState(hasTaskStarted) {
  return {
    threadId: null,
    currentTurnId: null,
    turnCounter: 0,
    pendingCalls: new Map(),
    hasTaskStarted,
  };
}

function ensureTurnOpen(state, ts, messages) {
  if (state.currentTurnId) return;
  state.turnCounter += 1;
  state.currentTurnId = `turn-${state.turnCounter}`;
  messages.push({
    ts,
    msg: {
      method: "turn/started",
      params: {
        threadId: state.threadId || "",
        turn: {
          id: state.currentTurnId,
          items: [],
          status: "inProgress",
          error: null,
        },
      },
    },
  });
}

function lineToMessages(state, line) {
  const messages = [];
  if (!line || typeof line !== "object") return messages;

  const eventType = line.type;
  const payload = line.payload || {};
  const eventTs = toEpochMs(payload.timestamp ?? line.timestamp ?? Date.now());

  if (eventType === "session_meta") {
    state.threadId = payload.id || null;
    messages.push({
      ts: eventTs,
      msg: {
        method: "thread/started",
        params: {
          thread: {
            id: payload.id || `thread-${Date.now()}`,
            preview: "",
            ephemeral: false,
            modelProvider: payload.model_provider || "openai",
            createdAt: Math.floor(toEpochMs(payload.timestamp) / 1000),
            updatedAt: Math.floor(toEpochMs(payload.timestamp) / 1000),
            status: "inProgress",
            path: null,
            cwd: payload.cwd || "",
            cliVersion: payload.cli_version || "",
            source: payload.source || "cli",
            agentNickname: null,
            agentRole: null,
            gitInfo: payload.git
              ? {
                  commitHash: payload.git.commit_hash || "",
                  branch: payload.git.branch || "",
                  repositoryUrl: payload.git.repository_url || "",
                }
              : null,
            name: null,
            turns: [],
          },
        },
      },
    });
    return messages;
  }

  if (eventType === "event_msg") {
    const subtype = payload.type;

    if (subtype === "task_started") {
      state.currentTurnId = payload.turn_id;
      state.hasTaskStarted = true;
      messages.push({
        ts: eventTs,
        msg: {
          method: "turn/started",
          params: {
            threadId: state.threadId || "",
            turn: {
              id: state.currentTurnId || "",
              items: [],
              status: "inProgress",
              error: null,
            },
          },
        },
      });
      return messages;
    }

    if (subtype === "user_message") {
      ensureTurnOpen(state, eventTs, messages);
      state.hasTaskStarted = true;
      messages.push({
        ts: eventTs,
        msg: {
          method: "item/completed",
          params: {
            threadId: state.threadId || "",
            turnId: state.currentTurnId || "",
            item: {
              type: "userMessage",
              id: `user-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
              content: [{ type: "text", text: payload.message || "" }],
            },
          },
        },
      });
      return messages;
    }

    if (subtype === "agent_message") {
      const phase = typeof payload.phase === "string" ? payload.phase : undefined;
      if (phase && phase !== "final_answer") {
        return messages;
      }

      const text = typeof payload.message === "string" ? payload.message.trim() : "";
      if (!text) return messages;

      ensureTurnOpen(state, eventTs, messages);
      messages.push({
        ts: eventTs,
        msg: {
          method: "item/completed",
          params: {
            threadId: state.threadId || "",
            turnId: state.currentTurnId || "",
            item: {
              type: "agentMessage",
              id: `agent-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
              text,
              ...(phase ? { phase } : {}),
            },
          },
        },
      });
      return messages;
    }

    if (subtype === "task_complete") {
      messages.push({
        ts: eventTs,
        msg: {
          method: "turn/completed",
          params: {
            threadId: state.threadId || "",
            turn: {
              id: payload.turn_id || state.currentTurnId || "",
              items: [],
              status: "completed",
              error: null,
            },
          },
        },
      });
      state.currentTurnId = null;
      return messages;
    }

    if (subtype === "turn_aborted") {
      messages.push({
        ts: eventTs,
        msg: {
          method: "turn/completed",
          params: {
            threadId: state.threadId || "",
            turn: {
              id: payload.turn_id || state.currentTurnId || "",
              items: [],
              status: "interrupted",
              error: null,
            },
          },
        },
      });
      state.currentTurnId = null;
      return messages;
    }

    return messages;
  }

  if (eventType === "turn_context") {
    if (!state.hasTaskStarted) {
      if (state.currentTurnId) {
        messages.push({
          ts: eventTs,
          msg: {
            method: "turn/completed",
            params: {
              threadId: state.threadId || "",
              turn: {
                id: state.currentTurnId,
                items: [],
                status: "completed",
                error: null,
              },
            },
          },
        });
      }
      state.turnCounter += 1;
      state.currentTurnId = `turn-${state.turnCounter}`;
      messages.push({
        ts: eventTs,
        msg: {
          method: "turn/started",
          params: {
            threadId: state.threadId || "",
            turn: {
              id: state.currentTurnId,
              items: [],
              status: "inProgress",
              error: null,
            },
          },
        },
      });
    }
    return messages;
  }

  if (eventType === "response_item") {
    const subtype = payload.type;

    if (subtype === "message") {
      // Do not map raw response messages to turn labels. In rollout files, this
      // stream includes bootstrap/system payloads (AGENTS/instructions,
      // turn_aborted wrappers, policy warnings) that are not user prompts.
      // Use event_msg:user_message as the source of truth instead.
      return messages;
    }

    if (subtype === "reasoning") {
      return messages;
    }

    if (subtype === "function_call") {
      if (payload.name === "update_plan") {
        const text = buildPlanTextFromArgs(payload.arguments || "");
        if (text) {
          ensureTurnOpen(state, eventTs, messages);
          messages.push({
            ts: eventTs,
            msg: {
              method: "item/completed",
              params: {
                threadId: state.threadId || "",
                turnId: state.currentTurnId || "",
                item: {
                  type: "plan",
                  id: payload.call_id || `plan-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
                  text,
                },
              },
            },
          });
        }
        return messages;
      }

      state.pendingCalls.set(payload.call_id, {
        name: payload.name,
        arguments: payload.arguments || "",
        timestamp: eventTs,
      });
      return messages;
    }

    if (subtype === "function_call_output") {
      const call = state.pendingCalls.get(payload.call_id);
      state.pendingCalls.delete(payload.call_id);
      if (!call) return messages;

      ensureTurnOpen(state, eventTs, messages);
      const name = call.name;
      const output = payload.output || "";
      let exitCode = 0;
      const exitMatch = /Process exited with code (\d+)/.exec(output);
      if (exitMatch) exitCode = Number(exitMatch[1]);

      if (name === "exec_command" || name === "shell" || name === "shell_command" || name === "write_stdin") {
        const { command, cwd } = extractCommand(call.arguments);
        messages.push({
          ts: eventTs,
          msg: {
            method: "item/completed",
            params: {
              threadId: state.threadId || "",
              turnId: state.currentTurnId || "",
              item: {
                type: "commandExecution",
                id: payload.call_id,
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
        return messages;
      }

      if (name === "apply_patch") {
        const filePaths = extractFilePaths(call.arguments);
        messages.push({
          ts: eventTs,
          msg: {
            method: "item/completed",
            params: {
              threadId: state.threadId || "",
              turnId: state.currentTurnId || "",
              item: {
                type: "fileChange",
                id: payload.call_id,
                changes: filePaths.map((path) => ({
                  path,
                  kind: { type: "update", move_path: null },
                  diff: call.arguments,
                })),
                status: exitCode === 0 ? "completed" : "failed",
              },
            },
          },
        });
        return messages;
      }

      let parsedArguments = {};
      try {
        parsedArguments = JSON.parse(call.arguments);
      } catch {
        // keep empty
      }

      messages.push({
        ts: eventTs,
        msg: {
          method: "item/completed",
          params: {
            threadId: state.threadId || "",
            turnId: state.currentTurnId || "",
            item: {
              type: "mcpToolCall",
              id: payload.call_id,
              server: "unknown",
              tool: name,
              status: exitCode === 0 ? "completed" : "failed",
              arguments: parsedArguments,
              result: { type: "text", text: output },
              error: null,
              durationMs: null,
            },
          },
        },
      });
      return messages;
    }

    if (subtype === "custom_tool_call") {
      state.pendingCalls.set(payload.call_id, {
        name: payload.name,
        input: payload.input || "",
        timestamp: eventTs,
      });
      return messages;
    }

    if (subtype === "custom_tool_call_output") {
      const call = state.pendingCalls.get(payload.call_id);
      state.pendingCalls.delete(payload.call_id);
      if (!call) return messages;

      ensureTurnOpen(state, eventTs, messages);
      const name = call.name;
      const output = payload.output || "";
      const parsedOutput = parseOutputJson(output);

      if (name === "apply_patch") {
        const filePaths = extractFilePaths(call.input);
        messages.push({
          ts: eventTs,
          msg: {
            method: "item/completed",
            params: {
              threadId: state.threadId || "",
              turnId: state.currentTurnId || "",
              item: {
                type: "fileChange",
                id: payload.call_id,
                changes: filePaths.map((path) => ({
                  path,
                  kind: { type: "update", move_path: null },
                  diff: call.input,
                })),
                status: parsedOutput?.metadata?.exit_code === 0 ? "completed" : "failed",
              },
            },
          },
        });
        return messages;
      }

      let parsedInput = {};
      try {
        parsedInput = typeof call.input === "string" ? { input: call.input } : call.input;
      } catch {
        // keep defaults
      }

      messages.push({
        ts: eventTs,
        msg: {
          method: "item/completed",
          params: {
            threadId: state.threadId || "",
            turnId: state.currentTurnId || "",
            item: {
              type: "mcpToolCall",
              id: payload.call_id,
              server: "custom",
              tool: name,
              status: parsedOutput?.metadata?.exit_code === 0 ? "completed" : "failed",
              arguments: parsedInput,
              result: { type: "text", text: parsedOutput?.output || output },
              error: null,
              durationMs: null,
            },
          },
        },
      });
      return messages;
    }
  }

  return messages;
}

function buildReplayMessages(rolloutLines) {
  const messages = [];
  const hasTaskStarted = rolloutLines.some(
    (line) => line?.type === "event_msg" && line?.payload?.type === "task_started",
  );
  const state = createReplayState(hasTaskStarted);

  for (const line of rolloutLines) {
    messages.push(...lineToMessages(state, line));
  }

  if (!hasTaskStarted && state.currentTurnId) {
    const lastLine = rolloutLines.at(-1);
    const finalTs = toEpochMs(lastLine?.timestamp ?? lastLine?.payload?.timestamp);
    messages.push({
      ts: finalTs,
      msg: {
        method: "turn/completed",
        params: {
          threadId: state.threadId || "",
          turn: {
            id: state.currentTurnId,
            items: [],
            status: "completed",
            error: null,
          },
        },
      },
    });
  }

  return { messages, state };
}

function parseRolloutFile(filePath) {
  const lines = readFileSync(filePath, "utf-8").split("\n").filter((line) => line.trim());
  return lines.map((line) => safeParse(line)).filter((line) => line !== null);
}

async function replayMessages(ws, messages, speed, includeDelay) {
  let prevTs = null;
  for (const { ts, msg } of messages) {
    if (ws.readyState !== 1) return;

    if (includeDelay && prevTs) {
      const deltaMs = (toEpochMs(ts) - toEpochMs(prevTs)) / speed;
      const waitMs = Math.min(Math.max(deltaMs, 0), 2000);
      if (waitMs > 0) {
        await sleep(waitMs);
      }
    }
    prevTs = ts;
    ws.send(JSON.stringify(msg));
  }
}

function createLineAppender(state, onMessage) {
  let carry = "";
  return function append(chunk) {
    if (!chunk) return;

    const text = carry + chunk;
    const lines = text.split("\n");
    carry = lines.pop() || "";

    for (const rawLine of lines) {
      const parsed = safeParse(rawLine);
      if (!parsed) continue;
      for (const item of lineToMessages(state, parsed)) {
        onMessage(item.msg);
      }
    }
  };
}

function startTail(filePath, state, onMessage, startOffset) {
  let cursor = Math.max(0, startOffset);
  let closed = false;
  const appendChunk = createLineAppender(state, onMessage);

  const poll = () => {
    if (closed) return;
    try {
      const content = readFileSync(filePath);
      const size = content.length;
      if (size < cursor) cursor = 0;
      if (size === cursor) return;
      appendChunk(content.slice(cursor).toString("utf-8"));
      cursor = size;
    } catch {
      // file might disappear briefly while being rotated
    }
  };

  const interval = setInterval(poll, WATCH_POLL_INTERVAL_MS);
  return () => {
    closed = true;
    clearInterval(interval);
  };
}

function fileSize(filePath) {
  try {
    return statSync(filePath).size;
  } catch {
    return 0;
  }
}

function toTrimmedString(value, maxLen = 4000) {
  if (typeof value !== "string") return "";
  const trimmed = value.trim();
  if (!trimmed) return "";
  return trimmed.slice(0, maxLen);
}

function toNullableString(value, maxLen = 4000) {
  const text = toTrimmedString(value, maxLen);
  return text || null;
}

function toStringList(value, maxItems = 64, maxLen = 400) {
  if (!Array.isArray(value)) return [];
  const out = [];
  for (const entry of value) {
    const text = toTrimmedString(entry, maxLen);
    if (!text) continue;
    if (!out.includes(text)) out.push(text);
    if (out.length >= maxItems) break;
  }
  return out;
}

function toCommands(value, maxItems = 64) {
  if (!Array.isArray(value)) return [];
  const out = [];
  for (const entry of value) {
    if (!entry || typeof entry !== "object") continue;
    const command = toTrimmedString(entry.command, 1000);
    if (!command) continue;
    const rawExitCode = entry.exitCode;
    const exitCode = rawExitCode === null || rawExitCode === undefined
      ? null
      : Number.isFinite(Number(rawExitCode))
        ? Number(rawExitCode)
        : null;
    out.push({ command, exitCode });
    if (out.length >= maxItems) break;
  }
  return out;
}

function normalizeSummaryStatus(rawStatus) {
  if (rawStatus === "success" || rawStatus === "completed") return "completed";
  if (rawStatus === "error" || rawStatus === "failed") return "failed";
  if (rawStatus === "cancelled") return "cancelled";
  return "in_progress";
}

function summarizeSignalFromInput(payload) {
  const hasCommandFailure = payload.commands.some((command) => command.exitCode !== null && command.exitCode !== 0);
  if (payload.status === "failed" || payload.errors.length > 0 || hasCommandFailure) return "error";
  if (payload.assistantMessage) return "agent_message";
  if (payload.commands.length > 0) return "command_activity";
  if (payload.filePaths.length > 0) return "file_change";
  return "status_only";
}

function summarizeTextFromInput(payload) {
  if (payload.assistantMessage) return payload.assistantMessage;
  const parts = [];
  if (payload.commands.length > 0) {
    parts.push(`${payload.commands.length} command${payload.commands.length === 1 ? "" : "s"}`);
  }
  if (payload.filePaths.length > 0) {
    parts.push(`${payload.filePaths.length} file${payload.filePaths.length === 1 ? "" : "s"} changed`);
  }
  if (payload.errors.length > 0) {
    parts.push(`${payload.errors.length} error${payload.errors.length === 1 ? "" : "s"}`);
  }
  return parts.join(" | ");
}

function parseSummaryRequest(payload) {
  if (!payload || typeof payload !== "object") return null;
  const turnId = toTrimmedString(payload.turnId, 256);
  if (!turnId) return null;

  return {
    turnId,
    sessionId: toNullableString(payload.sessionId, 256),
    status: normalizeSummaryStatus(toTrimmedString(payload.status, 64)),
    parentTurnId: toNullableString(payload.parentTurnId, 256),
    childTurnIds: toStringList(payload.childTurnIds, 64, 256),
    assistantMessage: toNullableString(payload.assistantMessage, 2000),
    commands: toCommands(payload.commands, 64),
    filePaths: toStringList(payload.filePaths, 128, 512),
    errors: toStringList(payload.errors, 64, 1000),
  };
}

function buildSummaryNodePayload(payload, modelSummary) {
  const summaryText = toTrimmedString(modelSummary?.summaryText, 180) || summarizeTextFromInput(payload);
  const fallbackSignal = summarizeSignalFromInput(payload);
  const signal = toTrimmedString(modelSummary?.signal, 40) || fallbackSignal;
  const primaryCommand = toNullableString(modelSummary?.primaryCommand, 1000)
    ?? payload.commands[payload.commands.length - 1]?.command
    ?? null;
  const primaryFilePath = toNullableString(modelSummary?.primaryFilePath, 512)
    ?? payload.filePaths[payload.filePaths.length - 1]
    ?? null;
  const primaryError = toNullableString(modelSummary?.primaryError, 1000)
    ?? payload.errors[payload.errors.length - 1]
    ?? null;
  const commandsIndexed = Math.min(payload.commands.length, 50);
  const filePathsIndexed = Math.min(payload.filePaths.length, 50);
  const errorsIndexed = Math.min(payload.errors.length, 50);

  return {
    nodeType: "turn",
    status: payload.status,
    summaryText,
    brief: {
      signal,
      agentMessage: payload.assistantMessage,
      primaryCommand,
      primaryFilePath,
      primaryError,
    },
    counts: {
      commandsTotal: payload.commands.length,
      commandsIndexed,
      commandsOmitted: payload.commands.length - commandsIndexed,
      filePathsTotal: payload.filePaths.length,
      filePathsIndexed,
      filePathsOmitted: payload.filePaths.length - filePathsIndexed,
      errorsTotal: payload.errors.length,
      errorsIndexed,
      errorsOmitted: payload.errors.length - errorsIndexed,
    },
    digest: {
      commandExamples: payload.commands.map((command) => command.command).slice(0, 3),
      filePathExamples: payload.filePaths.slice(0, 5),
      errorExamples: payload.errors.slice(0, 5),
    },
    lineage: {
      parentTurnId: payload.parentTurnId,
      childTurnId: payload.childTurnIds[payload.childTurnIds.length - 1] ?? null,
      childTurnIds: payload.childTurnIds,
      forkedFromThreadId: null,
      startedAfterRollback: false,
      wasRolledBack: payload.status === "cancelled",
    },
    evidence: {
      childTurnIds: payload.childTurnIds,
      filePaths: payload.filePaths,
      commands: payload.commands,
      errors: payload.errors,
    },
  };
}

function parseJsonSafe(input) {
  try {
    return JSON.parse(input);
  } catch {
    return null;
  }
}

function readJsonBody(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    let totalBytes = 0;
    req.on("data", (chunk) => {
      totalBytes += chunk.length;
      if (totalBytes > MAX_SUMMARY_BODY_BYTES) {
        reject(new Error("payload_too_large"));
        req.destroy();
        return;
      }
      chunks.push(chunk);
    });
    req.on("end", () => {
      const text = Buffer.concat(chunks).toString("utf-8");
      const parsed = parseJsonSafe(text);
      if (!parsed || typeof parsed !== "object") {
        reject(new Error("invalid_json"));
        return;
      }
      resolve(parsed);
    });
    req.on("error", reject);
  });
}

function sendJson(res, statusCode, payload) {
  res.writeHead(statusCode, { "Content-Type": "application/json" });
  res.end(JSON.stringify(payload));
}

async function summarizeWithOpenAi(payload) {
  const promptInput = {
    turnId: payload.turnId,
    sessionId: payload.sessionId,
    status: payload.status,
    parentTurnId: payload.parentTurnId,
    childTurnIds: payload.childTurnIds.slice(0, 12),
    assistantMessage: payload.assistantMessage,
    commands: payload.commands.slice(-12),
    filePaths: payload.filePaths.slice(0, 20),
    errors: payload.errors.slice(0, 12),
  };

  const response = await fetch(`${OPENAI_BASE_URL}/chat/completions`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${OPENAI_API_KEY}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      model: OPENAI_SUMMARY_MODEL,
      temperature: 0.2,
      response_format: { type: "json_object" },
      messages: [
        { role: "system", content: SUMMARY_OPENAI_SYSTEM_PROMPT },
        {
          role: "user",
          content: JSON.stringify(promptInput),
        },
      ],
    }),
  });

  const responseText = await response.text();
  if (!response.ok) {
    const preview = responseText.slice(0, 500);
    throw new Error(`openai_error_${response.status}:${preview}`);
  }

  const parsed = parseJsonSafe(responseText);
  const modelContent = parsed?.choices?.[0]?.message?.content;
  if (typeof modelContent !== "string") {
    return null;
  }
  const summaryObject = parseJsonSafe(modelContent);
  if (!summaryObject || typeof summaryObject !== "object") {
    return null;
  }
  return summaryObject;
}

async function handleSummarizeRequest(req, res) {
  if (SUMMARY_MODE !== "openai") {
    sendJson(res, 409, { error: "summary_mode_not_openai" });
    return;
  }
  if (!OPENAI_API_KEY) {
    sendJson(res, 503, { error: "missing_openai_api_key" });
    return;
  }

  let requestBody;
  try {
    requestBody = await readJsonBody(req);
  } catch (err) {
    if (err instanceof Error && err.message === "payload_too_large") {
      sendJson(res, 413, { error: "payload_too_large" });
      return;
    }
    sendJson(res, 400, { error: "invalid_request_json" });
    return;
  }

  const parsed = parseSummaryRequest(requestBody);
  if (!parsed) {
    sendJson(res, 400, { error: "invalid_summary_payload" });
    return;
  }

  let openAiSummary = null;
  try {
    openAiSummary = await summarizeWithOpenAi(parsed);
  } catch (err) {
    const message = err instanceof Error ? err.message : "summary_call_failed";
    sendJson(res, 502, { error: "openai_summary_failed", detail: message });
    return;
  }

  const node = buildSummaryNodePayload(parsed, openAiSummary);
  sendJson(res, 200, {
    turnId: parsed.turnId,
    node,
    provider: "openai",
    model: OPENAI_SUMMARY_MODEL,
  });
}

let defaultRolloutPath = INITIAL_FILE;
if (USE_LATEST && !defaultRolloutPath) {
  defaultRolloutPath = getLatestSessionFile();
  if (defaultRolloutPath) {
    console.log("Default rollout (latest):", defaultRolloutPath);
  }
}

const server = createServer((req, res) => {
  res.setHeader("Access-Control-Allow-Origin", "*");
  res.setHeader("Access-Control-Allow-Methods", "GET, POST, OPTIONS");
  res.setHeader("Access-Control-Allow-Headers", "Content-Type");

  if (req.method === "OPTIONS") {
    res.writeHead(204);
    res.end();
    return;
  }

  const requestUrl = new URL(req.url ?? "/", `http://localhost:${PORT}`);

  if (req.method === "GET" && requestUrl.pathname === "/api/sessions") {
    const sessions = getSessionList();
    sendJson(res, 200, sessions);
    return;
  }

  if (req.method === "POST" && requestUrl.pathname === "/api/summarize") {
    void handleSummarizeRequest(req, res);
    return;
  }

  res.writeHead(404);
  res.end("Not found");
});

const wss = new WebSocketServer({ server });

wss.on("connection", async (ws, req) => {
  const query = req.url ? new URL(req.url, `http://localhost:${PORT}`) : new URL("http://localhost");
  const fileFromQuery = query.searchParams.get("file");
  const watchEnabled = WATCH_MODE || query.searchParams.has("watch") || query.searchParams.has("auto");
  const trackLatest = watchEnabled && !fileFromQuery;
  let latestPoll = null;
  let stopTail = () => {};
  let isClosing = false;
  let openState = createReplayState(false);
  let activeSession = fileFromQuery ? resolve(fileFromQuery) : defaultRolloutPath;

  const stopEverything = () => {
    if (isClosing) return;
    isClosing = true;
    stopTail();
    if (latestPoll) {
      clearInterval(latestPoll);
      latestPoll = null;
    }
    stopTail = () => {};
  };

  const openSession = async (sessionPath) => {
    if (!sessionPath || !existsSync(sessionPath)) {
      return false;
    }
    const lines = parseRolloutFile(sessionPath);
    const { messages, state } = buildReplayMessages(lines);
    openState = state;
    stopTail();

    await replayMessages(ws, messages, SPEED, !watchEnabled);
    if (ws.readyState !== 1) return false;

    if (!watchEnabled) {
      return true;
    }

    stopTail = startTail(
      sessionPath,
      openState,
      (msg) => {
        if (ws.readyState === 1) {
          ws.send(JSON.stringify(msg));
        }
      },
      fileSize(sessionPath),
    );
    return true;
  };

  const openLatest = async () => {
    const latest = getLatestSessionFile();
    if (!latest || latest === activeSession) return false;
    const opened = await openSession(latest);
    if (opened) {
      activeSession = latest;
    }
    return opened;
  };

  if (!activeSession && !watchEnabled) {
    ws.close();
    return;
  }

  if (activeSession) {
    const opened = await openSession(activeSession);
    if (!opened && !trackLatest) {
      ws.close();
      return;
    }
  } else {
    await openLatest();
  }

  if (trackLatest) {
    latestPoll = setInterval(() => {
      if (ws.readyState !== 1) return;
      void openLatest();
    }, WATCH_POLL_INTERVAL_MS);
  }

  ws.once("close", stopEverything);
});

server.listen(PORT, () => {
  console.log(`Replay server running on http://localhost:${PORT}`);
  console.log(`  Sessions dir: ${SESSIONS_DIR}`);
  console.log("  GET /api/sessions — list available sessions");
  console.log(`  POST /api/summarize — OpenAI summary mode: ${SUMMARY_MODE}`);
  console.log(`  WS  ws://localhost:${PORT}[?watch=1|?file=<path>]`);
  console.log(`  Speed: ${watchModeLabel()}`);
});

function watchModeLabel() {
  return WATCH_MODE ? "live" : `${SPEED}x`;
}
