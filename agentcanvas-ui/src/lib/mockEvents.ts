import type { AppEvent } from './types'

export const MOCK_EVENTS: AppEvent[] = [
  // Session root
  { type: 'ThreadStarted', threadId: 'thr_agentcanvas_demo', ts: 900 },

  // Turn 1
  { type: 'TurnStarted', turnId: 't1', sessionId: 'thr_agentcanvas_demo', userPrompt: '(typing…)', ts: 1000 },
  { type: 'UserPromptPatch', turnId: 't1', userPrompt: 'List all TypeScript files in src/', ts: 1050 },
  { type: 'CommandExecution', id: 'e1', turnId: 't1', cmd: 'find src/ -name "*.ts"', exitCode: 0, stdout: 'src/main.ts\nsrc/App.tsx\nsrc/lib/types.ts', stderr: '', ts: 1100 },
  { type: 'PlanUpdate', id: 'e2', turnId: 't1', text: 'Found 3 TypeScript files — listing them for the user.', ts: 1200 },
  { type: 'TurnComplete', turnId: 't1', status: 'success', ts: 1300 },

  // Turn 2 — follows turn 1 in the conversation
  { type: 'TurnStarted', turnId: 't2', sessionId: 'thr_agentcanvas_demo', userPrompt: '(typing…)', ts: 2000 },
  { type: 'UserPromptPatch', turnId: 't2', userPrompt: 'Add a console.log to main.ts and run it', ts: 2050 },
  { type: 'McpToolCall', id: 'e3', turnId: 't2', toolName: 'read_file', server: 'filesystem', input: { path: 'src/main.ts' }, output: { content: 'import React from "react"' }, status: 'success', ts: 2100 },
  { type: 'PatchApply', id: 'e4', turnId: 't2', filename: 'src/main.ts', diff: '+console.log("hello from AgentCanvas")', status: 'success', ts: 2200 },
  { type: 'CommandExecution', id: 'e5', turnId: 't2', cmd: 'node src/main.ts', exitCode: 1, stdout: '', stderr: 'SyntaxError: Cannot use import statement outside a module', ts: 2300 },
  { type: 'TurnComplete', turnId: 't2', status: 'error', ts: 2400 },

  // Turn 3 — continues from turn 2's error
  { type: 'TurnStarted', turnId: 't3', sessionId: 'thr_agentcanvas_demo', userPrompt: '(typing…)', ts: 3000 },
  { type: 'UserPromptPatch', turnId: 't3', userPrompt: 'Fix the import error and use tsx instead', ts: 3050 },
  { type: 'PlanUpdate', id: 'e6', turnId: 't3', text: 'Will replace node with tsx to handle ESM imports correctly.', ts: 3100 },
  { type: 'CommandExecution', id: 'e7', turnId: 't3', cmd: 'npx tsx src/main.ts', exitCode: 0, stdout: 'hello from AgentCanvas', stderr: '', ts: 3200 },
  { type: 'TurnComplete', turnId: 't3', status: 'success', ts: 3300 },
]
