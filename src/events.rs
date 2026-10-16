//! Flatten records into a stream of small events with stable ids: what a ledger, a dashboard or a grep wants.
//!
//! Ids derive from the record uuid and block ordinal (never from parse time), so parsing the same bytes twice
//! yields the same ids and an append-only store can dedupe on them.

use crate::record::{content_text, prompt_kind, tool_input_summary, Block, Record, View};
use crate::util::{excerpt, fmt_ts, id, EXCERPT};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{json, Value};

/// Kinds: `prompt` (a person typed), `command` (/slash), `command_output`, `meta` (harness text), `tool_result`,
/// `assistant`, `thinking`, `tool_call`, `usage` (one per API message), `system`, `compact`, `queue`,
/// `attachment` (queued_command, edited_text_file, scheduled_task_fire, away_summary), `cost_state`, `artifact`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Event {
    pub event_id: String,
    pub ts: DateTime<Utc>,
    pub kind: &'static str,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub cwd: Option<String>,
    /// `promptId`: groups everything one prompt caused.
    pub turn_id: Option<String>,
    pub model: Option<String>,
    /// Correlation: tool_use id (on tool_call and tool_result), API message id (on usage).
    pub r#ref: Option<String>,
    pub tool: Option<String>,
    /// Excerpt (`EXCERPT` chars). Full text stays in the transcript at `src_offset`.
    pub text: Option<String>,
    pub bytes: Option<i64>,
    pub is_error: Option<bool>,
    pub tok_in: Option<i64>,
    pub tok_out: Option<i64>,
    pub tok_cache_read: Option<i64>,
    pub tok_cache_write: Option<i64>,
    pub tok_reasoning: Option<i64>,
    pub cost_usd: Option<f64>,
    pub src_offset: Option<i64>,
    pub src_len: Option<i64>,
    /// Source-specific JSON (origin, agent meta, durations).
    pub extra: Option<String>,
}

/// Session-level facts that have no timestamp of their own.
#[derive(Debug, Clone, Serialize)]
pub enum SessionSignal {
    /// `custom-title` / `agent-name`: the session's name (kk name, /rename).
    Name { session_id: String, name: String, kind: &'static str },
    /// A record with a session id, timestamp and cwd was seen (first/last seen, cwd).
    Seen { session_id: String, ts: DateTime<Utc>, cwd: Option<String> },
}

/// Per-file parse state: carries the session id and last timestamp across lines (some lines have neither).
#[derive(Debug, Clone, Default)]
pub struct Ctx {
    /// Path (or any label) used as the uuid fallback and reported nowhere else.
    pub src: String,
    pub agent_id: Option<String>,
    /// Subagent meta (agentType, description, model) → copied into `extra` of every event.
    pub agent_meta: Option<Value>,
    pub session_id: Option<String>,
    pub last_ts: Option<DateTime<Utc>>,
    pub excerpt_len: usize,
}

impl Ctx {
    pub fn new(src: impl Into<String>) -> Ctx { Ctx { src: src.into(), excerpt_len: EXCERPT, ..Default::default() } }
    pub fn for_agent(src: impl Into<String>, agent_id: impl Into<String>, meta: Value) -> Ctx {
        Ctx { agent_id: Some(agent_id.into()), agent_meta: (!meta.is_null()).then_some(meta), ..Ctx::new(src) }
    }
}

/// Events (and session signals) for one record, updating the context.
pub fn events(r: &Record, ctx: &mut Ctx) -> (Vec<Event>, Vec<SessionSignal>) {
    let mut out = vec![];
    let mut sig = vec![];
    let n = if ctx.excerpt_len == 0 { EXCERPT } else { ctx.excerpt_len };
    if let Some(t) = r.timestamp { ctx.last_ts = Some(t); }
    if let Some(s) = &r.session_id { ctx.session_id = Some(s.clone()); }
    let sid = r.session_id.clone().or_else(|| ctx.session_id.clone());
    let view = r.view();

    if let View::Title { kind, name } = &view {
        if ctx.agent_id.is_none() {
            if let Some(s) = &sid {
                if let Some(t) = ctx.last_ts { sig.push(SessionSignal::Seen { session_id: s.clone(), ts: t, cwd: None }); }
                sig.push(SessionSignal::Name { session_id: s.clone(), name: name.clone(), kind });
            }
        }
        return (out, sig);
    }
    let Some(ts) = r.timestamp.or(if r.kind == "cost-state" { ctx.last_ts } else { None }) else { return (out, sig) };
    if let (Some(s), None) = (&sid, &ctx.agent_id) {
        sig.push(SessionSignal::Seen { session_id: s.clone(), ts, cwd: r.cwd.clone() });
    }
    let uuid = r.uuid.clone().unwrap_or_else(|| format!("{}#{}", ctx.src, r.offset));
    let base = Event {
        ts,
        session_id: sid.clone(),
        agent_id: ctx.agent_id.clone().or_else(|| r.raw.get("agentId").and_then(|x| x.as_str()).map(str::to_string)),
        cwd: r.cwd.clone(),
        turn_id: r.raw.get("promptId").and_then(|x| x.as_str()).map(str::to_string),
        src_offset: Some(r.offset as i64),
        src_len: Some(r.len as i64),
        extra: ctx.agent_meta.as_ref().map(|m| json!({"agent_type": m.get("agentType"), "desc": m.get("description"), "model": m.get("model")}).to_string()),
        ..Default::default()
    };
    match view {
        View::User { message, is_meta, origin, .. } => {
            let origin = origin.map(|o| o.to_string());
            for (k, b) in message.content.iter().enumerate() {
                let mut e = base.clone();
                e.event_id = id(&["claude", &uuid, &k.to_string()]);
                match b {
                    Block::ToolResult { tool_use_id, content, is_error } => {
                        let body = content_text(content);
                        e.kind = "tool_result";
                        e.r#ref = tool_use_id.clone();
                        e.is_error = *is_error;
                        e.bytes = Some(content.to_string().len() as i64);
                        e.text = Some(excerpt(&body, 200));
                    }
                    other => {
                        let text = match other { Block::Text(t) => t.clone(), Block::Image => "[image]".into(), _ => continue };
                        e.kind = if is_meta { "meta" } else { prompt_kind(&text) };
                        e.bytes = Some(text.len() as i64);
                        e.text = Some(excerpt(&text, n));
                        e.extra = origin.clone().or(e.extra);
                    }
                }
                out.push(e);
            }
        }
        View::Assistant { message, request_id, .. } => {
            for (k, b) in message.content.iter().enumerate() {
                let mut e = base.clone();
                e.model = message.model.clone();
                e.event_id = id(&["claude", &uuid, &k.to_string()]);
                match b {
                    Block::Text(t) => { e.kind = "assistant"; e.bytes = Some(t.len() as i64); e.text = Some(excerpt(t, n)); }
                    Block::Thinking { len, .. } => { e.kind = "thinking"; e.bytes = Some(*len as i64); }
                    Block::ToolUse { id: tid, name, input, .. } => {
                        e.kind = "tool_call";
                        e.tool = Some(name.clone());
                        e.r#ref = tid.clone();
                        e.bytes = Some(input.to_string().len() as i64);
                        e.text = Some(tool_input_summary(input, n));
                    }
                    _ => continue,
                }
                out.push(e);
            }
            if let (Some(u), Some(m)) = (&message.usage, &message.model) {
                if m != "<synthetic>" {
                    let mid = message.id.clone().or(request_id).unwrap_or_else(|| uuid.clone());
                    let mut e = base.clone();
                    e.event_id = id(&["claude-usage", &mid]);
                    e.kind = "usage";
                    e.model = Some(m.clone());
                    e.r#ref = Some(mid);
                    e.tok_in = u.input_tokens;
                    e.tok_out = u.output_tokens;
                    e.tok_cache_read = u.cache_read_input_tokens;
                    e.tok_cache_write = u.cache_creation_input_tokens;
                    e.tok_reasoning = u.thinking_tokens;
                    out.push(e);
                }
            }
        }
        View::System { subtype, content, duration_ms, .. } => {
            let mut e = base.clone();
            e.event_id = id(&["claude", &uuid]);
            e.kind = if subtype == "compact_boundary" { "compact" } else { "system" };
            e.tool = Some(subtype);
            e.text = content.map(|t| excerpt(&t, 200));
            e.extra = duration_ms.map(|d| json!({"duration_ms": d}).to_string()).or(e.extra);
            out.push(e);
        }
        View::QueueOperation { operation, content, .. } if operation == "enqueue" => {
            let t = content.unwrap_or_default();
            let mut e = base.clone();
            e.event_id = id(&["claude", &uuid]);
            e.kind = "queue";
            e.bytes = Some(t.len() as i64);
            e.text = Some(excerpt(&t, n));
            out.push(e);
        }
        View::Attachment { attachment_type, attachment } => {
            if matches!(attachment_type.as_str(), "queued_command" | "edited_text_file" | "scheduled_task_fire" | "away_summary") {
                let t = ["prompt", "content", "filename", "summary"].iter().find_map(|k| attachment.get(k).and_then(|x| x.as_str())).unwrap_or("").to_string();
                let mut e = base.clone();
                e.event_id = id(&["claude", &uuid]);
                e.kind = "attachment";
                e.tool = Some(attachment_type);
                e.text = Some(excerpt(&t, n));
                out.push(e);
            }
        }
        View::CostState { total_cost_usd, api_ms, tool_ms, lines_added, lines_removed } => {
            let mut e = base.clone();
            e.event_id = id(&["claude-cost", sid.as_deref().unwrap_or(""), &format!("{:?}", total_cost_usd)]);
            e.kind = "cost_state";
            e.cost_usd = total_cost_usd;
            e.extra = Some(json!({"api_ms": api_ms, "tool_ms": tool_ms, "lines_added": lines_added, "lines_removed": lines_removed}).to_string());
            out.push(e);
        }
        View::FrameLink { title, url } => {
            let mut e = base.clone();
            e.event_id = id(&["claude-frame", url.as_deref().unwrap_or(""), &fmt_ts(&ts)]);
            e.kind = "artifact";
            e.text = Some(format!("{} {}", title.unwrap_or_default(), url.unwrap_or_default()));
            out.push(e);
        }
        _ => {}
    }
    for e in &mut out {
        if e.tool.is_none() && e.text.as_deref().is_some_and(|t| t.contains("<cross-session-message")) {
            e.tool = Some("cross_session".into());
        }
    }
    (out, sig)
}

/// Convenience: all events of a file (sessions and subagents alike).
pub fn file_events(path: &std::path::Path, ctx: &mut Ctx) -> std::io::Result<(Vec<Event>, Vec<SessionSignal>)> {
    let recs = crate::reader::read_records(path)?;
    let mut ev = vec![];
    let mut sg = vec![];
    for r in &recs {
        let (e, s) = events(r, ctx);
        ev.extend(e);
        sg.extend(s);
    }
    Ok((ev, sg))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rec(s: &str) -> Record { Record::parse(s.as_bytes(), 0) }
    #[test]
    fn prompt_tool_call_result_usage() {
        let mut ctx = Ctx::new("t");
        let (e, sig) = events(&rec(r#"{"type":"user","uuid":"u1","sessionId":"s1","timestamp":"2026-10-15T00:00:00Z","promptId":"p1","cwd":"/x","message":{"role":"user","content":"do it"}}"#), &mut ctx);
        assert_eq!(e.len(), 1);
        assert_eq!((e[0].kind, e[0].text.as_deref(), e[0].turn_id.as_deref()), ("prompt", Some("do it"), Some("p1")));
        assert!(matches!(&sig[0], SessionSignal::Seen { session_id, cwd: Some(c), .. } if session_id == "s1" && c == "/x"));
        let (e, _) = events(&rec(r#"{"type":"assistant","uuid":"a1","timestamp":"2026-10-15T00:00:01Z","message":{"id":"m1","model":"claude-opus-5-5","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls -la"}}],"usage":{"input_tokens":10,"output_tokens":2}}}"#), &mut ctx);
        assert_eq!(e.len(), 2);
        assert_eq!((e[0].kind, e[0].tool.as_deref(), e[0].r#ref.as_deref(), e[0].text.as_deref()), ("tool_call", Some("Bash"), Some("t1"), Some("command: ls -la")));
        assert_eq!((e[1].kind, e[1].tok_in, e[1].r#ref.as_deref()), ("usage", Some(10), Some("m1")));
        assert_eq!(e[0].session_id.as_deref(), Some("s1"), "session id carried by ctx");
        let (e, _) = events(&rec(r#"{"type":"user","uuid":"u2","timestamp":"2026-10-15T00:00:02Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"x","is_error":false}]}}"#), &mut ctx);
        assert_eq!((e[0].kind, e[0].r#ref.as_deref(), e[0].is_error), ("tool_result", Some("t1"), Some(false)));
    }
    #[test]
    fn ids_are_stable_and_usage_dedupes_per_message() {
        let mut c1 = Ctx::new("t");
        let mut c2 = Ctx::new("t");
        let line = r#"{"type":"assistant","uuid":"a1","timestamp":"2026-10-15T00:00:01Z","message":{"id":"m1","model":"claude-opus-5-5","content":[{"type":"text","text":"hi"}],"usage":{"input_tokens":1,"output_tokens":1}}}"#;
        let (e1, _) = events(&rec(line), &mut c1);
        let (e2, _) = events(&rec(line), &mut c2);
        assert_eq!(e1[0].event_id, e2[0].event_id);
        let line2 = line.replace("\"uuid\":\"a1\"", "\"uuid\":\"a2\"");
        let (e3, _) = events(&rec(&line2), &mut c1);
        assert_ne!(e1[0].event_id, e3[0].event_id, "text blocks differ by uuid");
        assert_eq!(e1[1].event_id, e3[1].event_id, "usage of the same API message collapses to one id");
    }
    #[test]
    fn titles_are_signals_not_events() {
        let mut ctx = Ctx::new("t");
        events(&rec(r#"{"type":"user","uuid":"u1","sessionId":"s1","timestamp":"2026-10-15T00:00:00Z","message":{"role":"user","content":"x"}}"#), &mut ctx);
        let (e, sig) = events(&rec(r#"{"type":"agent-name","sessionId":"s1","agentName":"lead"}"#), &mut ctx);
        assert!(e.is_empty());
        assert!(sig.iter().any(|s| matches!(s, SessionSignal::Name { name, kind: "agent-name", .. } if name == "lead")));
    }
    #[test]
    fn plumbing_is_classified() {
        let mut ctx = Ctx::new("t");
        let (e, _) = events(&rec(r#"{"type":"user","uuid":"u1","timestamp":"2026-10-15T00:00:00Z","message":{"role":"user","content":"<system-reminder>x</system-reminder>"}}"#), &mut ctx);
        assert_eq!(e[0].kind, "meta");
        let (e, _) = events(&rec(r#"{"type":"system","uuid":"s9","subtype":"compact_boundary","timestamp":"2026-10-15T00:00:00Z","content":"c"}"#), &mut ctx);
        assert_eq!(e[0].kind, "compact");
        let (e, _) = events(&rec(r#"{"type":"attachment","uuid":"at1","timestamp":"2026-10-15T00:00:00Z","attachment":{"type":"total_tokens_reminder"}}"#), &mut ctx);
        assert!(e.is_empty(), "noise attachments produce no event");
        let (e, _) = events(&rec(r#"{"type":"attachment","uuid":"at2","timestamp":"2026-10-15T00:00:00Z","attachment":{"type":"queued_command","prompt":"next"}}"#), &mut ctx);
        assert_eq!((e[0].kind, e[0].text.as_deref()), ("attachment", Some("next")));
    }
}
