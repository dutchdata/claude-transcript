//! A transcript as a thread: turns, messages, replies, reactions, nested subagents. Lossless: every node keeps
//! its record, and `render` prints everything (thinking, full tool inputs and results, attachments, usage).
//!
//! Structure comes from three links Claude Code writes:
//!
//! - `parentUuid`: the conversation chain (and branches after a resume or an interrupt);
//! - `tool_use_id` on a `tool_result`: the reply to a tool call;
//! - `promptId`: the turn a message belongs to (everything one prompt caused).
//!
//! Subagent transcripts (`<session>/subagents/agent-*.jsonl`) hang under the `Agent` tool call that spawned them
//! when their `agent-*.meta.json` or the tool result names the agent id; otherwise they are listed at the end.

use crate::record::{content_text, prompt_kind, Block, Record, View};
use crate::util::fmt_ts;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, Serialize)]
pub struct Thread {
    pub session_id: Option<String>,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub version: Option<String>,
    pub first_ts: Option<String>,
    pub last_ts: Option<String>,
    pub turns: Vec<Turn>,
    /// Subagents that could not be attached to a tool call.
    pub orphans: Vec<Subthread>,
    pub counts: Counts,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Counts {
    pub records: usize,
    pub turns: usize,
    pub prompts: usize,
    pub assistant_messages: usize,
    pub tool_calls: usize,
    pub tool_errors: usize,
    pub thinking_blocks: usize,
    pub compactions: usize,
    pub interrupts: usize,
    pub attachments: usize,
    pub unknown_records: usize,
}

/// One prompt and everything it caused, in file order.
#[derive(Debug, Clone, Serialize)]
pub struct Turn {
    pub turn_id: Option<String>,
    pub ts: Option<String>,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "item")]
#[allow(clippy::large_enum_variant)]
pub enum Item {
    Prompt { ts: Option<String>, uuid: Option<String>, text: String, kind: &'static str, origin: Option<Value>, reactions: Vec<Reaction> },
    Assistant { ts: Option<String>, uuid: Option<String>, model: Option<String>, text: String, thinking: Vec<Thinking>, reactions: Vec<Reaction> },
    ToolCall { ts: Option<String>, uuid: Option<String>, id: Option<String>, name: String, input: Value, reply: Option<ToolReply>, subthread: Option<Subthread>, reactions: Vec<Reaction> },
    System { ts: Option<String>, subtype: String, content: Option<String>, duration_ms: Option<i64> },
    Compaction { ts: Option<String>, content: Option<String>, meta: Option<Value> },
    Attachment { ts: Option<String>, attachment_type: String, attachment: Value },
    Queued { ts: Option<String>, operation: String, content: Option<String> },
    Usage { ts: Option<String>, model: Option<String>, message_id: Option<String>, input: Option<i64>, output: Option<i64>, cache_read: Option<i64>, cache_write: Option<i64>, thinking: Option<i64> },
    Cost { ts: Option<String>, total_usd: Option<f64> },
    Artifact { ts: Option<String>, title: Option<String>, url: Option<String> },
    Other { ts: Option<String>, kind: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct Thinking { pub text: Option<String>, pub len: usize, pub redacted: bool }

#[derive(Debug, Clone, Serialize)]
pub struct ToolReply { pub ts: Option<String>, pub uuid: Option<String>, pub content: Value, pub text: String, pub is_error: bool, pub structured: Option<Value> }

/// Things that happened *to* a message rather than in it.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "reaction")]
pub enum Reaction {
    /// The user interrupted while this was running.
    Interrupted { ts: Option<String>, text: String },
    /// A tool call was denied (permission) or refused.
    Denied { ts: Option<String>, kind: Option<String>, decision: Option<Value> },
    /// The tool result came back with is_error.
    Error { ts: Option<String> },
    /// `userFeedback` / `permissionDecision` fields the harness recorded on the message.
    Feedback { ts: Option<String>, value: Value },
    /// An API error on the assistant message.
    ApiError { ts: Option<String>, error: Value },
    /// The prompt was queued before it ran (origin says from where).
    Queued { reason: Option<String> },
}

#[derive(Debug, Clone, Serialize)]
pub struct Subthread { pub agent_id: String, pub agent_type: Option<String>, pub description: Option<String>, pub model: Option<String>, pub thread: Thread }

/// A subagent transcript to nest: its records and meta, keyed by agent id.
pub struct AgentInput { pub agent_id: String, pub meta: Value, pub records: Vec<Record> }

impl Thread {
    pub fn build(records: &[Record]) -> Thread { Thread::build_with_agents(records, Vec::new()) }

    pub fn build_with_agents(records: &[Record], agents: Vec<AgentInput>) -> Thread {
        let mut th = Thread { session_id: None, title: None, cwd: None, version: None, first_ts: None, last_ts: None, turns: vec![], orphans: vec![], counts: Counts::default() };
        th.counts.records = records.len();
        // Build subthreads first and index them by agent id.
        let mut subs: HashMap<String, Subthread> = HashMap::new();
        for a in agents {
            let t = Thread::build(&a.records);
            subs.insert(a.agent_id.clone(), Subthread {
                agent_id: a.agent_id,
                agent_type: a.meta.get("agentType").and_then(|x| x.as_str()).map(str::to_string),
                description: a.meta.get("description").and_then(|x| x.as_str()).map(str::to_string),
                model: a.meta.get("model").and_then(|x| x.as_str()).map(str::to_string),
                thread: t,
            });
        }
        // Pass 1: tool results and reactions, keyed by the tool_use id / the message they refer to.
        let mut replies: HashMap<String, ToolReply> = HashMap::new();
        let mut interrupts: Vec<(Option<String>, String, Option<String>)> = vec![]; // (ts, text, parent uuid)
        for r in records {
            if let View::User { message, tool_use_result, .. } = r.view() {
                for b in &message.content {
                    if let Block::ToolResult { tool_use_id: Some(tid), content, is_error } = b {
                        replies.insert(tid.clone(), ToolReply { ts: r.timestamp.map(|t| fmt_ts(&t)), uuid: r.uuid.clone(), text: content_text(content), content: content.clone(), is_error: is_error.unwrap_or(false), structured: tool_use_result.clone() });
                    }
                }
                let text = message.text();
                if text.trim_start().starts_with("[Request interrupted") {
                    interrupts.push((r.timestamp.map(|t| fmt_ts(&t)), text, r.parent_uuid.clone()));
                }
            }
        }
        // Pass 2: walk in file order, grouping by promptId (falling back to "a prompt starts a turn").
        let mut cur: Option<Turn> = None;
        let mut last_turn_id: Option<String> = None;
        let push_turn = |th: &mut Thread, cur: &mut Option<Turn>| { if let Some(t) = cur.take() { if !t.items.is_empty() { th.turns.push(t); } } };
        for r in records {
            let ts = r.timestamp.map(|t| fmt_ts(&t));
            if th.session_id.is_none() { th.session_id = r.session_id.clone(); }
            if th.cwd.is_none() { th.cwd = r.cwd.clone(); }
            if th.version.is_none() { th.version = r.version.clone(); }
            if let Some(t) = &ts { if th.first_ts.is_none() { th.first_ts = Some(t.clone()); } th.last_ts = Some(t.clone()); }
            let turn_id = r.raw.get("promptId").and_then(|x| x.as_str()).map(str::to_string);
            let view = r.view();
            let starts_turn = matches!(&view, View::User { message, is_meta: false, .. } if message.content.iter().all(|b| !matches!(b, Block::ToolResult { .. })) && prompt_kind(&message.text()) == "prompt");
            if (turn_id.is_some() && turn_id != last_turn_id) || (starts_turn && turn_id.is_none()) {
                push_turn(&mut th, &mut cur);
                cur = Some(Turn { turn_id: turn_id.clone(), ts: ts.clone(), items: vec![] });
                if turn_id.is_some() { last_turn_id = turn_id.clone(); }
            }
            let turn = cur.get_or_insert_with(|| Turn { turn_id: turn_id.clone(), ts: ts.clone(), items: vec![] });
            match view {
                View::Title { name, .. } => { th.title = Some(name); }
                View::User { message, is_meta, origin, .. } => {
                    let mut reactions = vec![];
                    if let Some(v) = r.raw.get("userFeedback") { reactions.push(Reaction::Feedback { ts: ts.clone(), value: v.clone() }); }
                    if let Some(k) = r.raw.get("toolDenialKind") { reactions.push(Reaction::Denied { ts: ts.clone(), kind: k.as_str().map(str::to_string), decision: r.raw.get("permissionDecision").cloned() }); }
                    if let Some(o) = &origin { if o.get("kind").is_some() || o.as_str().is_some_and(|s| s.contains("queue")) { reactions.push(Reaction::Queued { reason: o.get("kind").and_then(|x| x.as_str()).map(str::to_string) }); } }
                    let has_result = message.content.iter().any(|b| matches!(b, Block::ToolResult { .. }));
                    if !has_result {
                        let text = message.text();
                        if text.is_empty() { continue; }
                        let kind = if is_meta { "meta" } else { prompt_kind(&text) };
                        if kind == "prompt" { th.counts.prompts += 1; }
                        if text.trim_start().starts_with("[Request interrupted") { th.counts.interrupts += 1; }
                        turn.items.push(Item::Prompt { ts: ts.clone(), uuid: r.uuid.clone(), text, kind, origin, reactions });
                    }
                }
                View::Assistant { message, api_error, .. } => {
                    let mut thinking = vec![];
                    let mut text = String::new();
                    let mut reactions = vec![];
                    if let Some(e) = api_error { reactions.push(Reaction::ApiError { ts: ts.clone(), error: e }); }
                    for (ts2, t, parent) in &interrupts { if parent.as_deref() == r.uuid.as_deref() { reactions.push(Reaction::Interrupted { ts: ts2.clone(), text: t.clone() }); } }
                    for b in message.content {
                        match b {
                            Block::Text(t) => { if !text.is_empty() { text.push('\n'); } text.push_str(&t); }
                            Block::Thinking { text: tt, len, redacted } => { th.counts.thinking_blocks += 1; thinking.push(Thinking { text: tt, len, redacted }); }
                            Block::ToolUse { id, name, input, .. } => {
                                th.counts.tool_calls += 1;
                                let reply = id.as_ref().and_then(|i| replies.remove(i));
                                let mut rx = vec![];
                                if reply.as_ref().is_some_and(|x| x.is_error) { th.counts.tool_errors += 1; rx.push(Reaction::Error { ts: reply.as_ref().and_then(|x| x.ts.clone()) }); }
                                let subthread = if name == "Agent" || name == "Task" { agent_id_for(&input, reply.as_ref()).and_then(|aid| subs.remove(&aid)) } else { None };
                                turn.items.push(Item::ToolCall { ts: ts.clone(), uuid: r.uuid.clone(), id, name, input, reply, subthread, reactions: rx });
                            }
                            _ => {}
                        }
                    }
                    if !text.is_empty() || !thinking.is_empty() {
                        th.counts.assistant_messages += 1;
                        turn.items.push(Item::Assistant { ts: ts.clone(), uuid: r.uuid.clone(), model: message.model.clone(), text, thinking, reactions });
                    }
                    if let (Some(u), Some(m)) = (message.usage, message.model) {
                        turn.items.push(Item::Usage { ts: ts.clone(), model: Some(m), message_id: message.id, input: u.input_tokens, output: u.output_tokens, cache_read: u.cache_read_input_tokens, cache_write: u.cache_creation_input_tokens, thinking: u.thinking_tokens });
                    }
                }
                View::System { subtype, content, duration_ms, compact, .. } => {
                    if subtype == "compact_boundary" { th.counts.compactions += 1; turn.items.push(Item::Compaction { ts: ts.clone(), content, meta: compact }); }
                    else { turn.items.push(Item::System { ts: ts.clone(), subtype, content, duration_ms }); }
                }
                View::Attachment { attachment_type, attachment } => { th.counts.attachments += 1; turn.items.push(Item::Attachment { ts: ts.clone(), attachment_type, attachment }); }
                View::QueueOperation { operation, content, .. } => turn.items.push(Item::Queued { ts: ts.clone(), operation, content }),
                View::CostState { total_cost_usd, .. } => turn.items.push(Item::Cost { ts: ts.clone(), total_usd: total_cost_usd }),
                View::FrameLink { title, url } => turn.items.push(Item::Artifact { ts: ts.clone(), title, url }),
                View::Other => { th.counts.unknown_records += 1; if !SILENT.contains(&r.kind.as_str()) { turn.items.push(Item::Other { ts: ts.clone(), kind: r.kind.clone() }); } }
                View::Invalid => { th.counts.unknown_records += 1; turn.items.push(Item::Other { ts: ts.clone(), kind: "<invalid json>".into() }); }
            }
        }
        push_turn(&mut th, &mut cur);
        th.counts.turns = th.turns.len();
        let mut rest: Vec<Subthread> = subs.into_values().collect();
        rest.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
        th.orphans = rest;
        th
    }

    /// Everything, as text. Indentation nests subagents; nothing is elided except by `max_block` (0 = unlimited).
    pub fn render(&self, max_block: usize) -> String {
        let mut out = String::new();
        self.render_into(&mut out, 0, max_block);
        out
    }

    fn render_into(&self, out: &mut String, depth: usize, max: usize) {
        let pad = "  ".repeat(depth);
        let cut = |s: &str| -> String { if max > 0 && s.chars().count() > max { let mut t: String = s.chars().take(max).collect(); t.push_str(&format!("… [+{} chars]", s.chars().count() - max)); t } else { s.to_string() } };
        let ind = |s: &str, p: &str| -> String { s.lines().map(|l| format!("{p}    {l}")).collect::<Vec<_>>().join("\n") };
        let _ = writeln(out, &format!("{pad}# session {}{}{}", self.session_id.as_deref().unwrap_or("?"), self.title.as_ref().map(|t| format!(" \"{t}\"")).unwrap_or_default(), self.cwd.as_ref().map(|c| format!("  cwd {c}")).unwrap_or_default()));
        let _ = writeln(out, &format!("{pad}# {} → {}  turns {} prompts {} tool calls {} (errors {}) thinking blocks {} compactions {} interrupts {}", self.first_ts.as_deref().unwrap_or("?"), self.last_ts.as_deref().unwrap_or("?"), self.counts.turns, self.counts.prompts, self.counts.tool_calls, self.counts.tool_errors, self.counts.thinking_blocks, self.counts.compactions, self.counts.interrupts));
        for (i, t) in self.turns.iter().enumerate() {
            let _ = writeln(out, &format!("\n{pad}## turn {} {}{}", i + 1, t.ts.as_deref().unwrap_or(""), t.turn_id.as_ref().map(|x| format!("  [{x}]")).unwrap_or_default()));
            for it in &t.items {
                match it {
                    Item::Prompt { ts, text, kind, reactions, .. } => { let _ = writeln(out, &format!("{pad}{} {}:\n{}{}", short(ts), if *kind == "prompt" { "USER" } else { kind }, ind(&cut(text), &pad), rx(reactions, &pad))); }
                    Item::Assistant { ts, model, text, thinking, reactions, .. } => {
                        for th in thinking { let _ = writeln(out, &format!("{pad}{} THINKING ({} chars{}){}", short(ts), th.len, if th.redacted { ", redacted" } else { "" }, th.text.as_ref().map(|x| format!(":\n{}", ind(&cut(x), &pad))).unwrap_or_default())); }
                        if !text.is_empty() { let _ = writeln(out, &format!("{pad}{} ASSISTANT{}:\n{}{}", short(ts), model.as_ref().map(|m| format!(" [{m}]")).unwrap_or_default(), ind(&cut(text), &pad), rx(reactions, &pad))); }
                    }
                    Item::ToolCall { ts, name, input, reply, subthread, reactions, .. } => {
                        let _ = writeln(out, &format!("{pad}{} TOOL {}:\n{}{}", short(ts), name, ind(&cut(&pretty(input)), &pad), rx(reactions, &pad)));
                        if let Some(r) = reply { let _ = writeln(out, &format!("{pad}{} RESULT{}:\n{}", short(&r.ts), if r.is_error { " (error)" } else { "" }, ind(&cut(&r.text), &pad))); }
                        if let Some(s) = subthread { let _ = writeln(out, &format!("{pad}  ↳ agent {} {}{}", s.agent_id, s.agent_type.as_deref().unwrap_or(""), s.description.as_ref().map(|d| format!(" \"{d}\"")).unwrap_or_default())); s.thread.render_into(out, depth + 2, max); }
                    }
                    Item::System { ts, subtype, content, duration_ms } => { let _ = writeln(out, &format!("{pad}{} SYSTEM {}{}{}", short(ts), subtype, duration_ms.map(|d| format!(" {d} ms")).unwrap_or_default(), content.as_ref().map(|c| format!(": {}", cut(c))).unwrap_or_default())); }
                    Item::Compaction { ts, content, .. } => { let _ = writeln(out, &format!("{pad}{} ---- COMPACTION ----{}", short(ts), content.as_ref().map(|c| format!(" {}", cut(c))).unwrap_or_default())); }
                    Item::Attachment { ts, attachment_type, attachment } => { let _ = writeln(out, &format!("{pad}{} ATTACHMENT {}:\n{}", short(ts), attachment_type, ind(&cut(&pretty(attachment)), &pad))); }
                    Item::Queued { ts, operation, content } => { let _ = writeln(out, &format!("{pad}{} QUEUE {}{}", short(ts), operation, content.as_ref().map(|c| format!(": {}", cut(c))).unwrap_or_default())); }
                    Item::Usage { ts, model, input, output, cache_read, cache_write, thinking, .. } => { let _ = writeln(out, &format!("{pad}{} USAGE {} in {} out {} cache_read {} cache_write {}{}", short(ts), model.as_deref().unwrap_or(""), input.unwrap_or(0), output.unwrap_or(0), cache_read.unwrap_or(0), cache_write.unwrap_or(0), thinking.map(|t| format!(" thinking {t}")).unwrap_or_default())); }
                    Item::Cost { ts, total_usd } => { let _ = writeln(out, &format!("{pad}{} COST total ${:.4}", short(ts), total_usd.unwrap_or(0.0))); }
                    Item::Artifact { ts, title, url } => { let _ = writeln(out, &format!("{pad}{} ARTIFACT {} {}", short(ts), title.as_deref().unwrap_or(""), url.as_deref().unwrap_or(""))); }
                    Item::Other { ts, kind } => { let _ = writeln(out, &format!("{pad}{} ({kind})", short(ts))); }
                }
            }
        }
        for s in &self.orphans {
            let _ = writeln(out, &format!("\n{pad}## agent {} {} (not attached to a tool call)", s.agent_id, s.agent_type.as_deref().unwrap_or("")));
            s.thread.render_into(out, depth + 1, max);
        }
    }
}

/// Record types that are bookkeeping with nothing to show in a thread.
const SILENT: &[&str] = &["bridge-session", "last-prompt", "atis-latch", "mode", "permission-mode", "api-request-blob", "api-request", "api-request-shape", "file-history-snapshot", "file-history-delta", "artifact-autoreact-ledger", "artifact-comment-monitor", "worktree-state", "relocated", "ai-title"];

fn agent_id_for(input: &Value, reply: Option<&ToolReply>) -> Option<String> {
    // The Agent tool's result names the agent id ("agentId": …) in structured output, or in text as agent-<id>.
    if let Some(s) = reply.and_then(|r| r.structured.as_ref()) {
        for k in ["agentId", "agent_id", "id"] { if let Some(v) = s.get(k).and_then(|x| x.as_str()) { return Some(v.trim_start_matches("agent-").to_string()); } }
    }
    let text = reply.map(|r| r.text.as_str()).unwrap_or("");
    if let Some(i) = text.find("agent-") {
        let id: String = text[i + 6..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect();
        if id.len() >= 4 { return Some(id); }
    }
    input.get("name").and_then(|x| x.as_str()).map(str::to_string)
}

fn rx(r: &[Reaction], pad: &str) -> String {
    if r.is_empty() { return String::new(); }
    let mut s = String::new();
    for x in r {
        s.push_str(&format!("\n{pad}    ⟵ {}", match x {
            Reaction::Interrupted { text, .. } => format!("interrupted: {}", text.trim()),
            Reaction::Denied { kind, .. } => format!("denied{}", kind.as_ref().map(|k| format!(" ({k})")).unwrap_or_default()),
            Reaction::Error { .. } => "error".into(),
            Reaction::Feedback { value, .. } => format!("feedback {value}"),
            Reaction::ApiError { error, .. } => format!("api error {error}"),
            Reaction::Queued { reason } => format!("queued{}", reason.as_ref().map(|k| format!(" ({k})")).unwrap_or_default()),
        }));
    }
    s
}
fn short(ts: &Option<String>) -> String { ts.as_ref().map(|t| t.get(11..19).unwrap_or(t).to_string()).unwrap_or_else(|| "        ".into()) }
fn pretty(v: &Value) -> String { match v { Value::String(s) => s.clone(), Value::Null => String::new(), other => serde_json::to_string_pretty(other).unwrap_or_default() } }
fn writeln(out: &mut String, s: &str) -> std::fmt::Result { use std::fmt::Write; writeln!(out, "{s}") }

/// Group a session's turn ids → (first ts, items) quickly for callers that only need the index.
pub fn turn_index(th: &Thread) -> BTreeMap<usize, (Option<String>, usize)> {
    th.turns.iter().enumerate().map(|(i, t)| (i + 1, (t.ts.clone(), t.items.len()))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn recs(lines: &[&str]) -> Vec<Record> { lines.iter().enumerate().map(|(i, l)| Record::parse(l.as_bytes(), i as u64 * 100)).collect() }
    #[test]
    fn turns_replies_reactions_and_nesting() {
        let main = recs(&[
            r#"{"type":"agent-name","sessionId":"s1","agentName":"lead"}"#,
            r#"{"type":"user","uuid":"u1","sessionId":"s1","timestamp":"2026-10-15T00:00:00Z","promptId":"p1","message":{"role":"user","content":"please run ls"}}"#,
            r#"{"type":"assistant","uuid":"a1","parentUuid":"u1","timestamp":"2026-10-15T00:00:01Z","promptId":"p1","message":{"id":"m1","model":"claude-opus-5-5","content":[{"type":"thinking","thinking":"plan"},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}],"usage":{"input_tokens":5,"output_tokens":3}}}"#,
            r#"{"type":"user","uuid":"u2","parentUuid":"a1","timestamp":"2026-10-15T00:00:02Z","promptId":"p1","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"a.txt","is_error":true}]}}"#,
            r#"{"type":"assistant","uuid":"a2","parentUuid":"u2","timestamp":"2026-10-15T00:00:03Z","promptId":"p1","message":{"id":"m2","model":"claude-opus-5-5","content":[{"type":"tool_use","id":"t2","name":"Agent","input":{"prompt":"go","description":"helper"}}]}}"#,
            r#"{"type":"user","uuid":"u3","parentUuid":"a2","timestamp":"2026-10-15T00:00:04Z","promptId":"p1","toolUseResult":{"agentId":"ag1"},"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t2","content":"done by agent-ag1"}]}}"#,
            r#"{"type":"assistant","uuid":"a3","parentUuid":"u3","timestamp":"2026-10-15T00:00:05Z","promptId":"p1","message":{"id":"m3","model":"claude-opus-5-5","content":[{"type":"text","text":"working…"}]}}"#,
            r#"{"type":"user","uuid":"u4","parentUuid":"a3","timestamp":"2026-10-15T00:00:06Z","promptId":"p1","message":{"role":"user","content":"[Request interrupted by user]"}}"#,
            r#"{"type":"system","uuid":"s9","subtype":"compact_boundary","timestamp":"2026-10-15T00:00:07Z","content":"compacted"}"#,
            r#"{"type":"user","uuid":"u5","sessionId":"s1","timestamp":"2026-10-15T00:01:00Z","promptId":"p2","message":{"role":"user","content":"second"}}"#,
            r#"{"type":"bridge-session","sessionId":"s1","bridgeSessionId":"b"}"#,
        ]);
        let agent = recs(&[
            r#"{"type":"user","uuid":"x1","timestamp":"2026-10-15T00:00:04Z","message":{"role":"user","content":"go"}}"#,
            r#"{"type":"assistant","uuid":"x2","timestamp":"2026-10-15T00:00:04Z","message":{"id":"mx","model":"claude-opus-5-5","content":[{"type":"text","text":"ok"}]}}"#,
        ]);
        let th = Thread::build_with_agents(&main, vec![AgentInput { agent_id: "ag1".into(), meta: serde_json::json!({"agentType":"general-purpose","description":"helper"}), records: agent }]);
        assert_eq!(th.title.as_deref(), Some("lead"));
        assert_eq!(th.turns.len(), 2, "two prompt ids → two turns");
        assert_eq!(th.counts.prompts, 2);
        assert_eq!(th.counts.tool_calls, 2);
        assert_eq!(th.counts.tool_errors, 1);
        assert_eq!(th.counts.compactions, 1);
        assert_eq!(th.counts.interrupts, 1);
        let t1 = &th.turns[0];
        let bash = t1.items.iter().find_map(|i| match i { Item::ToolCall { name, reply, reactions, .. } if name == "Bash" => Some((reply.clone(), reactions.clone())), _ => None }).unwrap();
        assert!(bash.0.as_ref().unwrap().is_error && bash.0.unwrap().text == "a.txt");
        assert!(matches!(bash.1[0], Reaction::Error { .. }));
        let agent_call = t1.items.iter().find_map(|i| match i { Item::ToolCall { name, subthread, .. } if name == "Agent" => subthread.clone(), _ => None }).unwrap();
        assert_eq!(agent_call.agent_id, "ag1");
        assert_eq!(agent_call.thread.counts.prompts, 1);
        assert!(th.orphans.is_empty());
        let interrupted = t1.items.iter().any(|i| matches!(i, Item::Assistant { reactions, .. } if reactions.iter().any(|r| matches!(r, Reaction::Interrupted { .. }))));
        assert!(interrupted, "interrupt attaches to the message it followed");
        let text = th.render(0);
        assert!(text.contains("USER:") && text.contains("TOOL Bash") && text.contains("RESULT (error)") && text.contains("↳ agent ag1") && text.contains("COMPACTION") && text.contains("THINKING (4 chars)"));
        assert!(!text.contains("bridge-session"), "silent bookkeeping stays out of the render");
        let cutt = th.render(3);
        assert!(cutt.contains("[+"));
    }
}
