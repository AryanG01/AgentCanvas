#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

WS_PORT="${AGENTCANVAS_WS_PORT:-8080}"
VITE_PORT="${AGENTCANVAS_VITE_PORT:-5173}"
WATCH_URL="http://127.0.0.1:${VITE_PORT}/?autoconnect=1&watch=1"
WS_LOG="${TMPDIR:-/tmp}/agentcanvas-ws.log"
VITE_LOG="${TMPDIR:-/tmp}/agentcanvas-vite.log"

start_ws() {
  node "$PROJECT_ROOT/agentcanvas-ui/scripts/ws-replay.mjs" --watch --latest --port "$WS_PORT" \
    >"$WS_LOG" 2>&1 &
  WS_PID=$!
  echo "Started websocket replay on port $WS_PORT (pid $WS_PID)."
  echo "WebSocket logs: $WS_LOG"
}

start_vite() {
  (cd "$PROJECT_ROOT/agentcanvas-ui" && npm run dev -- --host 127.0.0.1 --port "$VITE_PORT") \
    >"$VITE_LOG" 2>&1 &
  VITE_PID=$!
  echo "Started agentcanvas UI dev server on port $VITE_PORT (pid $VITE_PID)."
  echo "Vite logs: $VITE_LOG"
}

open_browser() {
  echo "Opening $WATCH_URL"
  if command -v open >/dev/null 2>&1; then
    open "$WATCH_URL"
  elif command -v xdg-open >/dev/null 2>&1; then
    xdg-open "$WATCH_URL"
  else
    echo "No 'open' or 'xdg-open' found. Open this manually: $WATCH_URL"
  fi
}

cleanup() {
  local rc=$?
  if [[ -n "${WS_PID:-}" ]] && kill -0 "$WS_PID" 2>/dev/null; then
    kill "$WS_PID" 2>/dev/null || true
  fi
  if [[ -n "${VITE_PID:-}" ]] && kill -0 "$VITE_PID" 2>/dev/null; then
    kill "$VITE_PID" 2>/dev/null || true
  fi
  exit "$rc"
}

trap cleanup INT TERM

cd "$PROJECT_ROOT"
start_ws
start_vite
sleep 2
open_browser

echo
echo "Both services are running."
echo "Press Ctrl+C to stop all."

wait
