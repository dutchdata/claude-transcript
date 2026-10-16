# claude-transcript

Parse Claude Code session transcripts: the JSON Lines files under `~/.claude/projects/`.

One number: **3.3 GB of transcripts (421k records, 675 files) parse into 285k events in 7.4 s on an M1 MacBook Pro, 450 MB/s**, single-threaded.

What it does not do: it does not call any API, does not read your `~/.claude.json`, and does not modify transcripts.

```sh
cargo install claude-transcript          # the `cct` CLI
cct sessions                             # sessions and subagents per project
cct render ~/.claude/projects/<proj>/<session>.jsonl        # the whole session as a thread, lossless
cct events <file or dir> --kind tool_call                   # flat events: prompt, tool_call, tool_result, usage, compact…
cct usage --by model --since 2026-10-01                     # tokens per model, usage deduplicated per API message
cct usage --by tool                                         # which tools get called
cct drift                                                   # what a new Claude Code release added to the format
cct tail <live session.jsonl>                               # follow a running session
cct stats                                                   # size, records, parse speed
```

## Why

Claude Code keeps a complete record of every session: every prompt, every tool call and its result, token usage per
API message, compactions, subagents, queued messages. The format is undocumented, grows with every release (30
versions in one month in the corpus above), and mixes conversation with harness plumbing. If you want to know what
your sessions cost, what your agents did, or read a session back as a thread, you need a parser that is lossless,
tolerant and tells you when the format moved.

## The library

```rust
use claude_transcript::{reader::read_records, Thread, events::{events, Ctx}};

let recs = read_records(path)?;                     // Vec<Record>: raw JSON + typed View, never fails on a line
let thread = Thread::build(&recs);                  // turns → items (prompt, assistant, tool call + reply, …)
print!("{}", thread.render(0));                     // text, nothing elided

let mut ctx = Ctx::new(path.display().to_string());
for r in &recs {
    let (evs, signals) = events(r, &mut ctx);       // flat events with stable ids; session names/seen as signals
}
```

Layers:

| module | gives you |
|---|---|
| `reader` | `Lines` (complete lines with byte offsets; a half-written last line is left for next time) and `FileId::resume` (same file grown → resume at offset; rotated or rewritten → start over) |
| `record` | `Record { kind, uuid, parent_uuid, session_id, timestamp, cwd, version, raw }` and `View`: `User`, `Assistant`, `System`, `Attachment`, `QueueOperation`, `CostState`, `Title`, `FrameLink`, `Other`, `Invalid`. `Message`/`Block`/`Usage` for content. |
| `events` | `Event` stream with 15 kinds and ids derived from record uuid + block ordinal (parse twice, same ids; usage collapses per API message id) |
| `thread` | `Thread { turns: Vec<Turn> }`, each item with its reply (tool results attached to their calls), reactions (interrupts, denials, errors, API errors, feedback, queued), nested subagent threads, counts; `render(max_block)` |
| `usage` | `totals(&events, By::Model | Day | Session | Tool)` with cache-hit ratio |
| `drift` | `Survey` over a corpus → `Report` of record types, keys, block types, attachment types and system subtypes the crate does not know |
| `discover` | `find(root)` → sessions and subagents (`<session>/subagents/[workflows/<wf>/]agent-<id>.jsonl` + `.meta.json`) |

Everything typed keeps the raw `serde_json::Value`, so a field the crate does not model is still there.

## The thread model

Claude Code writes three links: `parentUuid` (conversation chain), `tool_use_id` on a `tool_result` (the reply to a
call), and `promptId` (the turn a message belongs to). `Thread::build` groups by turn, attaches each result to its
call, nests subagent transcripts under the `Agent` call that spawned them, and records as **reactions** the things
that happened *to* a message: `[Request interrupted]` on the message that was running, `is_error` on a result,
`toolDenialKind`/`permissionDecision`, `apiError`, `userFeedback`, and queued origin. Harness plumbing (system
reminders, environment attachments) is kept and labeled, not dropped; bookkeeping records with nothing to show
(`bridge-session`, `last-prompt`, `api-request-blob`, …) are counted but not rendered.

## Format drift

`cct drift` surveys a corpus and prints what this version of the crate has not seen: new record types, new top-level
keys on modeled types, new content block types, new attachment types, new system subtypes, and the Claude Code
versions present. The lists it checks against are in `src/drift.rs`; `KNOWN_CLAUDE_CODE_VERSION` says what the crate
was last verified against. FORMAT.md documents the format as observed.

## Guarantees

- Never panics on input: bad JSON becomes a `Record` with `kind == "<invalid>"`; unknown types become `View::Other`.
- Lossless: `raw` is the line.
- Stable event ids: same bytes, same ids, across runs and machines.
- A transcript being written by a live session is safe to read: only complete lines are consumed.
- No network, no writes.

## License

MIT or Apache-2.0, at your option. Built by [DutchData](https://dutchdata.io), where it feeds Ledger, our record of
everything the agents did.
