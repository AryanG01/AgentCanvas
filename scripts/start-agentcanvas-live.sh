#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

load_env_file() {
  local env_file="$1"
  if [[ -f "$env_file" ]]; then
    while IFS= read -r raw_line || [[ -n "$raw_line" ]]; do
      local line="$raw_line"
      line="${line#"${line%%[![:space:]]*}"}"
      line="${line%"${line##*[![:space:]]}"}"
      [[ -z "$line" || "$line" == \#* ]] && continue

      if [[ "$line" =~ ^export[[:space:]]+ ]]; then
        line="${line#export }"
      fi

      if [[ "$line" =~ ^([A-Za-z_][A-Za-z0-9_]*)=(.*)$ ]]; then
        local key="${BASH_REMATCH[1]}"
        local value="${BASH_REMATCH[2]}"
        if [[ "$value" =~ ^\"(.*)\"$ ]]; then
          value="${BASH_REMATCH[1]}"
        elif [[ "$value" =~ ^\'(.*)\'$ ]]; then
          value="${BASH_REMATCH[1]}"
        fi
        export "$key=$value"
        continue
      fi

      if [[ "$line" =~ ^sk-[A-Za-z0-9._-]+$ ]]; then
        export OPENAI_API_KEY="$line"
        echo "Warning: inferred OPENAI_API_KEY from bare key line in $env_file"
        continue
      fi

      echo "Warning: skipped invalid env line in $env_file: $line"
    done < "$env_file"
    echo "Loaded env: $env_file"
  fi
}

load_env_file "$PROJECT_ROOT/.env.local"
load_env_file "$PROJECT_ROOT/agentcanvas-ui/.env.local"

WS_PORT="${AGENTCANVAS_WS_PORT:-8080}"
VITE_PORT="${AGENTCANVAS_VITE_PORT:-5173}"
WATCH_URL="http://127.0.0.1:${VITE_PORT}/?autoconnect=1&watch=1"
WS_LOG="${TMPDIR:-/tmp}/agentcanvas-ws.log"
VITE_LOG="${TMPDIR:-/tmp}/agentcanvas-vite.log"
SUMMARY_MODE="${AGENTCANVAS_SUMMARY_MODE:-local}"
SUMMARY_MODEL="${AGENTCANVAS_SUMMARY_MODEL:-gpt-4o-mini}"
SUMMARY_REMOTE="false"
if [[ "$SUMMARY_MODE" == "openai" ]]; then
  SUMMARY_REMOTE="true"
fi
SUMMARY_API_URL="http://127.0.0.1:${WS_PORT}/api/summarize"

if [[ "$SUMMARY_MODE" == "openai" && -z "${AGENTCANVAS_OPENAI_API_KEY:-}" && -z "${OPENAI_API_KEY:-}" ]]; then
  echo "Warning: AGENTCANVAS_SUMMARY_MODE=openai but no OPENAI_API_KEY/AGENTCANVAS_OPENAI_API_KEY is set."
  echo "         The UI will fall back to local summaries."
fi

start_ws() {
  AGENTCANVAS_SUMMARY_MODE="$SUMMARY_MODE" \
  AGENTCANVAS_SUMMARY_MODEL="$SUMMARY_MODEL" \
  node "$PROJECT_ROOT/agentcanvas-ui/scripts/ws-replay.mjs" --watch --latest --port "$WS_PORT" \
    >"$WS_LOG" 2>&1 &
  WS_PID=$!
  echo "Started websocket replay on port $WS_PORT (pid $WS_PID)."
  echo "Summary backend mode: $SUMMARY_MODE (model: $SUMMARY_MODEL)"
  echo "Summary endpoint: $SUMMARY_API_URL"
  echo "WebSocket logs: $WS_LOG"
}

start_vite() {
  (
    cd "$PROJECT_ROOT/agentcanvas-ui" \
    && VITE_SUMMARY_REMOTE="$SUMMARY_REMOTE" \
       VITE_SUMMARY_API_URL="$SUMMARY_API_URL" \
       npm run dev -- --host 127.0.0.1 --port "$VITE_PORT"
  ) \
    >"$VITE_LOG" 2>&1 &
  VITE_PID=$!
  echo "Started agentcanvas UI dev server on port $VITE_PORT (pid $VITE_PID)."
  echo "UI summary remote mode: $SUMMARY_REMOTE"
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
