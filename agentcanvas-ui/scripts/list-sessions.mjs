#!/usr/bin/env node

/**
 * list-sessions.mjs — List available Codex rollout sessions.
 *
 * Scans the rollout sessions directory (~/.codex/sessions) and displays a summary
 * of each session: date, turn count, source, working directory, and file path.
 *
 * Usage: node scripts/list-sessions.mjs [--limit 20]
 */

import { readFileSync, readdirSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const args = process.argv.slice(2);
const argValue = (name) => {
  const idx = args.indexOf(name);
  return idx === -1 ? undefined : args[idx + 1];
};

const limitRaw = argValue("--limit");
const LIMIT = Number.isFinite(Number(limitRaw)) ? Number(limitRaw) : 20;
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

function readSessionInfo(filePath) {
  try {
    const content = readFileSync(filePath, "utf-8");
    const lines = content.split("\n").filter((l) => l.trim());

    let meta = null;
    let taskStartedCount = 0;
    let turnContextCount = 0;

    for (const line of lines) {
      try {
        const obj = JSON.parse(line);
        if (obj.type === "session_meta" && !meta) {
          meta = obj.payload;
        }
        if (obj.type === "event_msg" && obj.payload?.type === "task_started") {
          taskStartedCount++;
        }
        if (obj.type === "turn_context") {
          turnContextCount++;
        }
      } catch {
        // skip malformed lines
      }
    }

    // Use task_started count if available, otherwise fall back to turn_context
    const turnCount = taskStartedCount > 0 ? taskStartedCount : turnContextCount;

    if (!meta) return null;

    const ts = new Date(meta.timestamp);
    const date = ts.toISOString().slice(0, 10);
    const time = ts.toISOString().slice(11, 16);

    return {
      date,
      time,
      turnCount,
      source: meta.source || "unknown",
      cwd: meta.cwd || "",
      filePath,
      id: meta.id,
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
