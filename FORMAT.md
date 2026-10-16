# The Claude Code transcript format, as observed

Observed in 675 files written by Claude Code 2.1.233 through 2.1.301 (October 2026). This is a description of what
is there, not a specification; Anthropic does not document it. `cct drift` tells you what has changed since.

## Files

- `~/.claude/projects/<project-slug>/<session-uuid>.jsonl`: one session. The slug is the working directory with `/`
  replaced by `-` (`/Users/you/src/x` → `-Users-you-src-x`). `CLAUDE_CONFIG_DIR` moves the root.
- `.../<session-uuid>/subagents/agent-<id>.jsonl` and `.../subagents/workflows/<run>/agent-<id>.jsonl`: one file per
  subagent (the `Agent` tool, workflows), with `agent-<id>.meta.json` beside it: `{agentType, description, model, …}`.
- Files are append-only while the session runs; a live file can end in a partial line.

## Lines

Every line is a JSON object with a `type`. Most conversation lines also carry `uuid`, `parentUuid`, `sessionId`,
`timestamp` (RFC 3339), `cwd`, `gitBranch`, `version` (Claude Code version), `isSidechain`, `userType`, `entrypoint`,
`slug`; agent lines add `agentId` and `sessionKind`.

### Conversation

| type | what | notable fields |
|---|---|---|
| `user` | a human prompt, a tool result, or harness text injected as a user message | `message.content` is a string (typed prompt) or blocks (`text`, `image`, `tool_result{tool_use_id, content, is_error}`); `promptId` (turn); `isMeta`; `isCompactSummary`; `origin`/`promptSource` (queued? from where); `toolUseResult` (structured result); `toolDenialKind`, `permissionDecision`; `userFeedback`; `interruptedMessageId` |
| `assistant` | one API message, split over several lines (one per content block); the same `message.id` and `usage` repeat on each | `message.model`, `message.content` blocks `text`, `thinking{thinking}`, `redacted_thinking{data}`, `tool_use{id, name, input}`, `server_tool_use`, `fallback`; `message.usage{input_tokens, output_tokens, cache_read_input_tokens, cache_creation_input_tokens, output_tokens_details.thinking_tokens}`; `requestId`; `apiError`/`error`; `effort`, `perTurnEffort`; `attribution*` (which skill/plugin/MCP server led to the call) |
| `system` | harness events | `subtype`: `turn_duration` (`durationMs`), `local_command`, `compact_boundary` (`compactMetadata`), `api_error` (`retryAttempt`, `retryInMs`), `away_summary`, `scheduled_task_fire`, `stop_hook_summary`, `model_refusal_no_fallback`, `model_refusal_fallback` (`fallbackModel`), `informational`, `bridge_status`; `content`; `level` |
| `attachment` | context attached to a turn | `attachment.type`: `environment`, `date`, `model`, `instructions`, `output_style`, `skill_listing`, `deferred_tools_record/delta`, `mcp_instructions_delta`, `agent_listing_delta`, `total_tokens_reminder`, `batching_reminder_sent`, `silent_turn_reminder`, `queued_command{prompt}`, `edited_text_file{filename}`, `file`, `prompt_snapshot`, `session_context`, `compact_file_reference`, `plan_mode[_exit]`, `auto_mode[_exit]`, `ultra_effort_enter/exit`, `hook_*`, `task_reminder`, `task_status`, `skill_mention`, `structured_output`, … |
| `queue-operation` | the prompt queue | `operation`: `enqueue` (with `content`), `dequeue`, `remove`, …; `reason`, `deliveryId` |

### Session dimension (no timestamp)

| type | fields |
|---|---|
| `custom-title` | `customTitle`: the name the user gave the session |
| `agent-name` | `agentName`: the harness/kk name |
| `ai-title` | `aiTitle` |
| `mode`, `permission-mode` | current modes |
| `last-prompt` | `lastPrompt`, `leafUuid`, `explicit` |
| `bridge-session` | remote-control bridge ids |
| `atis-latch` | `atis` |
| `relocated`, `worktree-state` | path moves, worktree info |

### Accounting and bookkeeping

| type | fields |
|---|---|
| `cost-state` | `totalCostUSD`, `totalAPIDuration`, `totalAPIDurationWithoutRetries`, `totalToolDuration`, `totalDuration`, `totalLinesAdded`, `totalLinesRemoved`, `modelUsage`, `hasUnknownModelCost`, `startTime`; no timestamp (use the previous line's) |
| `frame-link` | an artifact the session published: `frameUrl`, `title`, `path`, `artifactCount` |
| `api-request`, `api-request-shape`, `api-request-blob` | request-shape hashes and blobs (`hash`, `shapeHash`, `requestDigest`) |
| `file-history-snapshot`, `file-history-delta` | file edit history for undo (`messageId`, `snapshot`) |
| `artifact-autoreact-ledger`, `artifact-comment-monitor` | artifact watch state |
| `started`, `launched`, `result`, `failed` | background task / workflow lifecycle |

## Links

- `parentUuid` → the previous message in the chain; branches appear after resumes and interrupts.
- `tool_result.tool_use_id` → the `tool_use.id` it answers. Results arrive as `user` lines.
- `promptId` → the turn; every assistant line, tool result and attachment caused by one prompt shares it.
- `message.id` → one API message across its several `assistant` lines; count usage once per id.
- `agent-<id>` in `Agent` results and `toolUseResult.agentId` → the subagent file.

## Harness text that looks like a prompt

User-role text starting with `<command-name>`/`<command-message>` is a slash command; `<local-command-stdout>` its
output; `<system-reminder>`, `<task-notification>`, `<cross-session-message>` and `[Request interrupted …]` are
harness messages. `prompt_kind()` classifies them; `isMeta` marks injected messages explicitly.

## Volumes (this corpus)

421k records: 20 % attachments, 14 % assistant lines, 8 % user lines, the rest session dimension and bookkeeping.
Among content blocks: tool_use 30k, tool_result 30k, thinking 21k, text 9k. Tools by call count: Bash 39k, Read 5.7k,
Edit 4.1k, Write 2.7k, SendMessage 651, WebFetch 516, WebSearch 431.
