//! One transcript line as a [`Record`]: the raw JSON plus a typed [`View`] of the parts the crate understands.
//! Nothing is dropped: fields the view does not model are still in `raw`.

use crate::util::{b, i, s, ts_value};
use chrono::{DateTime, Utc};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct Record {
    /// Byte offset of the line in its file (0-based), and its length without the newline.
    pub offset: u64,
    pub len: u64,
    /// The `type` field, or `"<invalid>"` when the line is not JSON / not an object.
    pub kind: String,
    pub uuid: Option<String>,
    pub parent_uuid: Option<String>,
    pub session_id: Option<String>,
    pub timestamp: Option<DateTime<Utc>>,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    /// Claude Code version that wrote the line (`version`).
    pub version: Option<String>,
    pub is_sidechain: bool,
    pub raw: Value,
}

/// What a record is, in the shapes that matter. `Other` keeps the type name for everything the crate only passes through.
#[derive(Debug, Clone)]
pub enum View {
    /// A human turn, a tool result, or harness plumbing injected as a user message.
    User { message: Message, is_meta: bool, is_compact_summary: bool, prompt_id: Option<String>, origin: Option<Value>, tool_use_result: Option<Value> },
    Assistant { message: Message, request_id: Option<String>, api_error: Option<Value> },
    /// `system` records: `subtype` is turn_duration, local_command, compact_boundary, api_error, stop_hook_summary, …
    System { subtype: String, content: Option<String>, duration_ms: Option<i64>, level: Option<String>, compact: Option<Value> },
    /// Context the harness attached to a turn (environment, reminders, queued commands, edited files, …).
    Attachment { attachment_type: String, attachment: Value },
    /// A queued prompt (`operation`: enqueue, dequeue, remove, …).
    QueueOperation { operation: String, content: Option<String>, reason: Option<String> },
    CostState { total_cost_usd: Option<f64>, api_ms: Option<i64>, tool_ms: Option<i64>, lines_added: Option<i64>, lines_removed: Option<i64> },
    /// `custom-title` (the user named the session) or `agent-name` (kk / /rename).
    Title { kind: &'static str, name: String },
    FrameLink { title: Option<String>, url: Option<String> },
    /// Lines the crate does not model: bridge-session, last-prompt, mode, permission-mode, api-request*, file-history-*, …
    Other,
    Invalid,
}

#[derive(Debug, Clone, Default)]
pub struct Message {
    pub role: Option<String>,
    pub model: Option<String>,
    /// API message id (`message.id`); several assistant lines can share one (one per content block).
    pub id: Option<String>,
    pub stop_reason: Option<String>,
    pub content: Vec<Block>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone)]
pub enum Block {
    Text(String),
    /// Thinking text may be present, redacted (`redacted_thinking`) or stripped; `len` is what is there.
    Thinking { text: Option<String>, len: usize, redacted: bool },
    ToolUse { id: Option<String>, name: String, input: Value, server: bool },
    /// `is_error` is `None` when the field is absent (older records), `Some(false)` when explicitly not an error.
    ToolResult { tool_use_id: Option<String>, content: Value, is_error: Option<bool> },
    Image,
    Other(Value),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Usage {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_input_tokens: Option<i64>,
    pub cache_creation_input_tokens: Option<i64>,
    pub thinking_tokens: Option<i64>,
}

impl Record {
    /// Parse one line (without its trailing newline). Never fails: bad JSON gives `kind == "<invalid>"`.
    pub fn parse(line: &[u8], offset: u64) -> Record {
        let raw: Value = serde_json::from_slice(line).unwrap_or(Value::Null);
        let obj = raw.is_object();
        let kind = if obj { s(&raw, "type").unwrap_or_else(|| "<none>".into()) } else { "<invalid>".into() };
        Record {
            offset,
            len: line.len() as u64,
            kind,
            uuid: s(&raw, "uuid"),
            parent_uuid: s(&raw, "parentUuid"),
            session_id: s(&raw, "sessionId").or_else(|| s(&raw, "session_id")),
            timestamp: raw.get("timestamp").and_then(ts_value),
            cwd: s(&raw, "cwd"),
            git_branch: s(&raw, "gitBranch"),
            version: s(&raw, "version"),
            is_sidechain: b(&raw, "isSidechain").unwrap_or(false),
            raw,
        }
    }

    pub fn view(&self) -> View {
        let v = &self.raw;
        match self.kind.as_str() {
            "<invalid>" => View::Invalid,
            "user" => View::User {
                message: Message::from_value(v.get("message").unwrap_or(&Value::Null)),
                is_meta: b(v, "isMeta").unwrap_or(false),
                is_compact_summary: b(v, "isCompactSummary").unwrap_or(false),
                prompt_id: s(v, "promptId"),
                origin: v.get("origin").or_else(|| v.get("promptSource")).cloned(),
                tool_use_result: v.get("toolUseResult").cloned(),
            },
            "assistant" => View::Assistant {
                message: Message::from_value(v.get("message").unwrap_or(&Value::Null)),
                request_id: s(v, "requestId"),
                api_error: v.get("apiError").or_else(|| v.get("error")).cloned(),
            },
            "system" => View::System {
                subtype: s(v, "subtype").unwrap_or_default(),
                content: s(v, "content"),
                duration_ms: i(v, "durationMs"),
                level: s(v, "level"),
                compact: v.get("compactMetadata").cloned(),
            },
            "attachment" => View::Attachment {
                attachment_type: v.pointer("/attachment/type").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                attachment: v.get("attachment").cloned().unwrap_or(Value::Null),
            },
            "queue-operation" => View::QueueOperation { operation: s(v, "operation").unwrap_or_default(), content: s(v, "content"), reason: s(v, "reason") },
            "cost-state" => View::CostState {
                total_cost_usd: v.get("totalCostUSD").and_then(|x| x.as_f64()),
                api_ms: i(v, "totalAPIDuration"),
                tool_ms: i(v, "totalToolDuration"),
                lines_added: i(v, "totalLinesAdded"),
                lines_removed: i(v, "totalLinesRemoved"),
            },
            "custom-title" => s(v, "customTitle").map(|name| View::Title { kind: "custom-title", name }).unwrap_or(View::Other),
            "agent-name" => s(v, "agentName").map(|name| View::Title { kind: "agent-name", name }).unwrap_or(View::Other),
            "frame-link" => View::FrameLink { title: s(v, "title"), url: s(v, "frameUrl") },
            _ => View::Other,
        }
    }
}

impl Message {
    pub fn from_value(m: &Value) -> Message {
        let content = match m.get("content") {
            Some(Value::String(t)) => vec![Block::Text(t.clone())],
            Some(Value::Array(a)) => a.iter().map(Block::from_value).collect(),
            _ => vec![],
        };
        Message {
            role: s(m, "role"),
            model: s(m, "model"),
            id: s(m, "id"),
            stop_reason: s(m, "stop_reason"),
            content,
            usage: m.get("usage").filter(|u| u.is_object()).map(|u| Usage {
                input_tokens: i(u, "input_tokens"),
                output_tokens: i(u, "output_tokens"),
                cache_read_input_tokens: i(u, "cache_read_input_tokens"),
                cache_creation_input_tokens: i(u, "cache_creation_input_tokens"),
                thinking_tokens: u.pointer("/output_tokens_details/thinking_tokens").and_then(|x| x.as_i64()),
            }),
        }
    }

    /// All text blocks joined with newlines; images become `[image]`. Tool blocks are excluded.
    pub fn text(&self) -> String {
        let mut parts = vec![];
        for b in &self.content {
            match b {
                Block::Text(t) => parts.push(t.as_str()),
                Block::Image => parts.push("[image]"),
                _ => {}
            }
        }
        parts.join("\n")
    }
}

impl Block {
    pub fn from_value(b: &Value) -> Block {
        match b.get("type").and_then(|x| x.as_str()).unwrap_or("") {
            "text" => Block::Text(s(b, "text").unwrap_or_default()),
            "thinking" => {
                let t = s(b, "thinking");
                Block::Thinking { len: t.as_ref().map(|x| x.len()).unwrap_or(0), text: t, redacted: false }
            }
            "redacted_thinking" => Block::Thinking { text: None, len: s(b, "data").map(|x| x.len()).unwrap_or(0), redacted: true },
            "tool_use" | "server_tool_use" => Block::ToolUse {
                id: s(b, "id"),
                name: s(b, "name").unwrap_or_default(),
                input: b.get("input").cloned().unwrap_or(Value::Null),
                server: b.get("type").and_then(|x| x.as_str()) == Some("server_tool_use"),
            },
            "tool_result" => Block::ToolResult { tool_use_id: s(b, "tool_use_id"), content: b.get("content").cloned().unwrap_or(Value::Null), is_error: b.get("is_error").and_then(|x| x.as_bool()) },
            "image" => Block::Image,
            _ => Block::Other(b.clone()),
        }
    }
}

/// Text of a tool result's `content` (a string, or blocks of text/images).
pub fn content_text(c: &Value) -> String {
    match c {
        Value::String(t) => t.clone(),
        Value::Array(a) => a
            .iter()
            .filter_map(|x| x.get("text").and_then(|t| t.as_str()).map(str::to_string).or_else(|| (x.get("type")?.as_str()? == "image").then(|| "[image]".to_string())))
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Human prompt vs. harness plumbing that arrives as user text.
pub fn prompt_kind(text: &str) -> &'static str {
    let t = text.trim_start();
    if t.starts_with("<command-name>") || t.starts_with("<command-message>") {
        "command"
    } else if t.starts_with("<local-command-stdout>") || t.starts_with("<local-command-stderr>") || t.starts_with("<local-command-caveat>") {
        "command_output"
    } else if t.starts_with("<system-reminder>") || t.starts_with("<task-notification>") || t.starts_with("<cross-session-message") || t.starts_with("[Request interrupted") {
        "meta"
    } else {
        "prompt"
    }
}

/// The one argument of a tool call worth showing in a line (`command: …`, `file_path: …`).
pub fn tool_input_summary(input: &Value, n: usize) -> String {
    for k in ["command", "file_path", "pattern", "url", "query", "prompt", "description", "skill", "path"] {
        if let Some(x) = input.get(k).and_then(|x| x.as_str()) {
            return crate::util::excerpt(&format!("{k}: {x}"), n);
        }
    }
    crate::util::excerpt(&input.to_string(), n)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_lines_do_not_fail() {
        let r = Record::parse(b"not json", 7);
        assert_eq!(r.kind, "<invalid>");
        assert!(matches!(r.view(), View::Invalid));
        assert_eq!(r.offset, 7);
    }
    #[test]
    fn user_prompt_and_tool_result() {
        let r = Record::parse(br#"{"type":"user","uuid":"u1","sessionId":"s1","timestamp":"2026-10-15T00:00:00Z","promptId":"p1","message":{"role":"user","content":"hi there"}}"#, 0);
        match r.view() {
            View::User { message, prompt_id, .. } => { assert_eq!(message.text(), "hi there"); assert_eq!(prompt_id.as_deref(), Some("p1")); }
            _ => panic!(),
        }
        let r = Record::parse(br#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"ok"}],"is_error":true}]}}"#, 0);
        match r.view() {
            View::User { message, .. } => match &message.content[0] {
                Block::ToolResult { tool_use_id, content, is_error } => { assert_eq!(tool_use_id.as_deref(), Some("t1")); assert_eq!(content_text(content), "ok"); assert_eq!(*is_error, Some(true)); }
                _ => panic!(),
            },
            _ => panic!(),
        }
    }
    #[test]
    fn assistant_blocks_and_usage() {
        let r = Record::parse(br#"{"type":"assistant","requestId":"req_1","message":{"id":"msg_1","model":"claude-fable-5-1","content":[{"type":"thinking","thinking":"hmm"},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}},{"type":"text","text":"done"}],"usage":{"input_tokens":3,"output_tokens":5,"cache_read_input_tokens":100,"output_tokens_details":{"thinking_tokens":2}}}}"#, 0);
        match r.view() {
            View::Assistant { message, request_id, .. } => {
                assert_eq!(request_id.as_deref(), Some("req_1"));
                assert_eq!(message.content.len(), 3);
                assert!(matches!(message.content[0], Block::Thinking { len: 3, .. }));
                assert!(matches!(&message.content[1], Block::ToolUse { name, .. } if name == "Bash"));
                assert_eq!(message.usage.as_ref().unwrap().thinking_tokens, Some(2));
                assert_eq!(message.text(), "done");
            }
            _ => panic!(),
        }
    }
    #[test]
    fn prompt_kinds() {
        assert_eq!(prompt_kind("<command-name>/x</command-name>"), "command");
        assert_eq!(prompt_kind("  <system-reminder>x"), "meta");
        assert_eq!(prompt_kind("[Request interrupted by user]"), "meta");
        assert_eq!(prompt_kind("<local-command-stdout>x"), "command_output");
        assert_eq!(prompt_kind("hello"), "prompt");
    }
    #[test]
    fn other_types_pass_through() {
        let r = Record::parse(br#"{"type":"bridge-session","sessionId":"s","bridgeSessionId":"b"}"#, 0);
        assert!(matches!(r.view(), View::Other));
        assert_eq!(r.raw["bridgeSessionId"], "b");
        let r = Record::parse(br#"{"type":"custom-title","sessionId":"s","customTitle":"lead"}"#, 0);
        assert!(matches!(r.view(), View::Title { kind: "custom-title", name } if name == "lead"));
    }
}
