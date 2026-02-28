#!/usr/bin/env node

/**
 * list-sessions.mjs — List available Codex rollout sessions.
 *
 * Scans ~/.codex/sessions/ for rollout JSONL files and displays a summary
 * of each session: date, turn count, source, working directory, and file path.
 *
 * Usage: node scripts/list-sessions.mjs [--limit 20]
 */

import { readFileSync, readdirSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const args = process.argv.slice(2);
const limitIdx = args.indexOf("--limit");
const LIMIT = limitIdx !== -1 ? Number(args[limitIdx + 1]) : 20;

const SESSIONS_DIR = join(homedir(), ".codex", "sessions");

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
        // skip malformed lines
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
      const turnCount = taskStartedCount > 0 ? taskStartedCount : turnContextCount;
      const { date, time } = formatLocalDateTime(meta.timestamp);
      return {
        date,
        time,
        turnCount,
        source: meta.source || "unknown",
        cwd: meta.cwd || "",
        filePath,
        id: meta.id,
      };
    }

    const header = parsed[0];
    if (!header?.id || !header?.timestamp) return null;

    let turnCount = 0;
    let cwd = "";
    for (const obj of parsed) {
      if (obj.type === "message" && obj.role === "user") {
        const text = extractMessageText(obj.content);
        if (text && !isSetupMessage(text)) turnCount++;
        if (!cwd && text) cwd = extractCwdFromText(text);
      }
      if (!cwd && obj.type === "function_call") {
        const args = parseArgumentsObject(obj.arguments);
        if (typeof args?.workdir === "string") cwd = args.workdir;
      }
    }

    const { date, time } = formatLocalDateTime(header.timestamp);
    return {
      date,
      time,
      turnCount,
      source: "cli",
      cwd,
      filePath,
      id: header.id,
    };
  } catch {
    return null;
  }
}

const allRollouts = findAllRollouts(SESSIONS_DIR).sort().reverse();

if (allRollouts.length === 0) {
  console.log(`No rollout files found in ${SESSIONS_DIR}`);
  process.exit(0);
}

const sessions = [];
for (const f of allRollouts) {
  if (sessions.length >= LIMIT) break;
  const info = readSessionInfo(f);
  if (info) sessions.push(info);
}

console.log(`Found ${allRollouts.length} rollout file(s), showing ${sessions.length}:\n`);

for (const s of sessions) {
  const turns = s.turnCount === 1 ? "1 turn " : `${s.turnCount} turns`;
  console.log(`  ${s.date} ${s.time}  ${turns.padEnd(9)} ${s.source.padEnd(8)} ${s.cwd}`);
  console.log(`    ${s.filePath.replace(homedir(), "~")}`);
}
