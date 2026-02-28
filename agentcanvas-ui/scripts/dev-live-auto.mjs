#!/usr/bin/env node

import { spawn } from "node:child_process";
import { EOL } from "node:os";
import process from "node:process";

const children = [];
let openedUrl = null;
let shuttingDown = false;

function log(msg) {
  process.stdout.write(`[dev:live:auto] ${msg}${EOL}`);
}

function openBrowser(url) {
  if (openedUrl) return;
  openedUrl = url;

  const platform = process.platform;
  let cmd = null;
  let args = [];

  if (platform === "darwin") {
    cmd = "open";
    args = [url];
  } else if (platform === "win32") {
    cmd = "cmd";
    args = ["/c", "start", "", url];
  } else {
    cmd = "xdg-open";
    args = [url];
  }

  const opener = spawn(cmd, args, { stdio: "ignore", detached: true });
  opener.on("error", (err) => {
    log(`failed to open browser automatically: ${err.message}`);
  });
  opener.unref();
  log(`opening ${url}`);
}

function pipeOutput(child, name) {
  function writeWithPrefix(data) {
    const lines = data.toString().split(/\r?\n/);
    for (const line of lines) {
      if (!line) continue;
      process.stdout.write(`[${name}] ${line}${EOL}`);

      if (name === "dev" && !openedUrl) {
        const m = line.match(/(https?:\/\/localhost:\d+\/?)/);
        if (m?.[1]) openBrowser(m[1]);
      }
    }
  }

  child.stdout?.on("data", writeWithPrefix);
  child.stderr?.on("data", writeWithPrefix);
}

function start(name, args) {
  const child = spawn(args[0], args.slice(1), {
    stdio: ["ignore", "pipe", "pipe"],
    env: process.env,
  });
  children.push(child);
  pipeOutput(child, name);

  child.on("exit", (code, signal) => {
    if (shuttingDown) return;
    const reason = signal ? `signal ${signal}` : `code ${code ?? 0}`;
    log(`${name} exited with ${reason}`);
    shutdown(code ?? 1);
  });
}

function shutdown(code = 0) {
  if (shuttingDown) return;
  shuttingDown = true;
  for (const child of children) {
    if (!child.killed) child.kill("SIGTERM");
  }
  setTimeout(() => {
    for (const child of children) {
      if (!child.killed) child.kill("SIGKILL");
    }
    process.exit(code);
  }, 750);
}

process.on("SIGINT", () => shutdown(0));
process.on("SIGTERM", () => shutdown(0));

log("starting replay server and Vite dev server");
start("replay", ["pnpm", "replay"]);
start("dev", ["pnpm", "dev"]);
