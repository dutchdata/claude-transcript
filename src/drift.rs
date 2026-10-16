//! Schema drift: what a corpus contains versus what this crate knows. Run it after a Claude Code update; new
//! record types, keys, block types, attachment types and system subtypes show up here before they break anything.

use crate::record::Record;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Record types the crate models or deliberately passes through (as of `KNOWN_CLAUDE_CODE_VERSION`).
pub const KNOWN_TYPES: &[&str] = &[
    "user", "assistant", "system", "attachment", "queue-operation", "cost-state", "custom-title", "agent-name", "frame-link",
    "bridge-session", "last-prompt", "atis-latch", "mode", "permission-mode", "api-request-blob", "api-request", "api-request-shape",
    "file-history-snapshot", "file-history-delta", "artifact-autoreact-ledger", "artifact-comment-monitor", "worktree-state", "relocated", "ai-title",
    // background-task lifecycle records (workflows / background agents)
    "started", "launched", "result", "failed",
];
/// Record types the crate reads fields from; new keys are reported for these only.
pub const MODELED_TYPES: &[&str] = &["user", "assistant", "system", "attachment", "queue-operation", "cost-state", "custom-title", "agent-name", "frame-link"];
pub const KNOWN_BLOCKS: &[&str] = &["text", "thinking", "redacted_thinking", "tool_use", "server_tool_use", "tool_result", "image", "fallback"];
pub const KNOWN_SYSTEM_SUBTYPES: &[&str] = &["turn_duration", "local_command", "away_summary", "scheduled_task_fire", "informational", "compact_boundary", "stop_hook_summary", "model_refusal_no_fallback", "model_refusal_fallback", "api_error", "bridge_status"];
pub const KNOWN_ATTACHMENTS: &[&str] = &[
    "total_tokens_reminder", "output_style", "batching_reminder_sent", "environment", "bash_output_audience_note", "deferred_tools_record", "silent_turn_reminder",
    "queued_command", "edited_text_file", "prompt_snapshot", "deferred_tools_delta", "mcp_instructions_delta", "date", "model", "remote_session_change", "instructions",
    "agent_listing_delta", "output_style_instructions", "session_context", "auto_mode", "credential_org", "skill_listing", "file", "compact_file_reference", "hook_success",
    "thinking_drop", "task_reminder", "command_permissions", "task_status", "ultra_effort_enter", "inlined_image_paths", "invoked_skills", "plan_mode_exit",
    "hook_additional_context", "hook_system_message", "read_truncation_notice", "plan_mode", "nested_memory", "auto_mode_exit", "ultra_effort_exit", "session_cron_carry", "plan_file_reference", "skill_mention", "structured_output",
];
/// Top-level keys seen per record type (what the crate reads or knowingly ignores). Anything else is reported.
pub fn known_keys(kind: &str) -> Vec<&'static str> {
    const COMMON: &[&str] = &["type", "uuid", "parentUuid", "sessionId", "session_id", "timestamp", "cwd", "gitBranch", "version", "isSidechain", "userType", "entrypoint", "slug", "isMeta", "requestId", "logicalParentUuid", "agentId", "sessionKind"];
    let specific: &[&str] = match kind {
        "user" => &["message", "promptId", "origin", "promptSource", "toolUseResult", "isCompactSummary", "isVisibleInTranscriptOnly", "imagePasteIds", "interruptedMessageId", "mcpMeta", "permissionDecision", "permissionMode", "queueOrigin", "queuePriority", "queueSkipAttachments", "queueSkipSlashCommands", "queueTranscriptOnly", "refusedMessageId", "scheduledFireId", "scheduledTaskId", "serverClassifierContext", "sourceToolAssistantUUID", "sourceToolUseID", "summarizedTurnPromptUuids", "toolDenialKind", "turnCompanion", "turnOrigin", "turnPosition", "userFeedback", "classifierMetaLines", "toolEndsTurn"],
        "assistant" => &["message", "apiError", "apiErrorParams", "apiErrorStatus", "isApiErrorMessage", "error", "errorDetails", "effort", "perTurnEffort", "advisorModel", "apiBlockIndex", "attributionMcpServer", "attributionMcpTool", "attributionPlugin", "attributionSkill", "quotaLimits", "refusedMessageId", "requestRef", "requestedModel", "serverClassifierRequest", "thinkingDisplay", "thinkingDurationMs", "truncatedAfterOutput", "wireIngestContext", "wireToolInputs", "attributionAgent"],
        "system" => &["subtype", "content", "durationMs", "level", "compactMetadata", "apiRefusalCategory", "apiRefusalExplanation", "commandRun", "contextUsage", "cron", "error", "hasOutput", "hookAdditionalContext", "hookCount", "hookErrors", "hookInfos", "maxRetries", "messageCount", "originalModel", "pendingBackgroundAgentCount", "pendingWorkflowCount", "preventedContinuation", "prompt", "refusedUserMessageUuid", "retryAttempt", "retryInMs", "source", "stopReason", "taskId", "toolUseID", "url", "direction", "fallbackModel", "retractedMessageUuids", "scope", "trigger"],
        "attachment" => &["attachment", "rendered", "renderedBesideToolResult", "renderedInHumanTurn", "renderedRole"],
        "queue-operation" => &["operation", "content", "reason", "commandUuid", "deliveryId"],
        "cost-state" => &["totalCostUSD", "totalAPIDuration", "totalToolDuration", "totalLinesAdded", "totalLinesRemoved", "hasUnknownModelCost", "modelUsage", "startTime", "totalAPIDurationWithoutRetries", "totalDuration"],
        "custom-title" => &["customTitle"],
        "agent-name" => &["agentName"],
        "frame-link" => &["frameUrl", "title", "artifactCount", "path"],
        "bridge-session" => &["bridgeSessionId", "lastSequenceNum", "ownerAccountUuid", "ownerOrganizationUuid"],
        "last-prompt" => &["explicit", "lastPrompt", "leafUuid"],
        "atis-latch" => &["atis"],
        "mode" => &["mode"],
        "permission-mode" => &["permissionMode"],
        "api-request-blob" => &["hash", "message", "agentId"],
        "file-history-snapshot" => &["isSnapshotUpdate", "messageId", "snapshot"],
        _ => &[],
    };
    specific.iter().chain(COMMON.iter()).copied().collect()
}

#[derive(Debug, Default, Serialize)]
pub struct Survey {
    pub files: usize,
    pub records: usize,
    pub bytes: u64,
    pub invalid_lines: usize,
    pub versions: BTreeMap<String, usize>,
    pub types: BTreeMap<String, usize>,
    pub keys: BTreeMap<String, BTreeSet<String>>,
    pub blocks: BTreeMap<String, usize>,
    pub attachments: BTreeMap<String, usize>,
    pub system_subtypes: BTreeMap<String, usize>,
    pub tools: BTreeMap<String, usize>,
    pub models: BTreeMap<String, usize>,
}

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub new_types: Vec<String>,
    pub new_keys: BTreeMap<String, Vec<String>>,
    pub new_blocks: Vec<String>,
    pub new_attachments: Vec<String>,
    pub new_system_subtypes: Vec<String>,
    pub versions: Vec<String>,
}

impl Survey {
    pub fn add(&mut self, r: &Record) {
        self.records += 1;
        self.bytes += r.len + 1;
        if r.kind == "<invalid>" { self.invalid_lines += 1; return; }
        if let Some(v) = &r.version { *self.versions.entry(v.clone()).or_default() += 1; }
        *self.types.entry(r.kind.clone()).or_default() += 1;
        if let Some(o) = r.raw.as_object() {
            let ks = self.keys.entry(r.kind.clone()).or_default();
            for k in o.keys() { ks.insert(k.clone()); }
        }
        if r.kind == "user" || r.kind == "assistant" {
            if let Some(m) = r.raw.pointer("/message/model").and_then(|x| x.as_str()) { *self.models.entry(m.to_string()).or_default() += 1; }
            match r.raw.pointer("/message/content") {
                Some(serde_json::Value::Array(a)) => for b in a {
                    let t = b.get("type").and_then(|x| x.as_str()).unwrap_or("?");
                    *self.blocks.entry(t.to_string()).or_default() += 1;
                    if t == "tool_use" { if let Some(n) = b.get("name").and_then(|x| x.as_str()) { *self.tools.entry(n.to_string()).or_default() += 1; } }
                },
                Some(serde_json::Value::String(_)) => *self.blocks.entry("<string>".into()).or_default() += 1,
                _ => {}
            }
        }
        if r.kind == "attachment" { *self.attachments.entry(r.raw.pointer("/attachment/type").and_then(|x| x.as_str()).unwrap_or("?").to_string()).or_default() += 1; }
        if r.kind == "system" { *self.system_subtypes.entry(r.raw.get("subtype").and_then(|x| x.as_str()).unwrap_or("?").to_string()).or_default() += 1; }
    }

    /// What is in the corpus that the crate's known lists do not have.
    pub fn report(&self) -> Report {
        let mut rep = Report { versions: self.versions.keys().cloned().collect(), ..Default::default() };
        rep.new_types = self.types.keys().filter(|t| !KNOWN_TYPES.contains(&t.as_str())).cloned().collect();
        rep.new_blocks = self.blocks.keys().filter(|t| t.as_str() != "<string>" && !KNOWN_BLOCKS.contains(&t.as_str())).cloned().collect();
        rep.new_attachments = self.attachments.keys().filter(|t| !KNOWN_ATTACHMENTS.contains(&t.as_str())).cloned().collect();
        rep.new_system_subtypes = self.system_subtypes.keys().filter(|t| !KNOWN_SYSTEM_SUBTYPES.contains(&t.as_str())).cloned().collect();
        for (kind, ks) in &self.keys {
            let known = known_keys(kind);
            let fresh: Vec<String> = ks.iter().filter(|k| !known.contains(&k.as_str())).cloned().collect();
            // Keys are only tracked for types with a known key list; pass-through types without one are not reported.
            if !fresh.is_empty() && MODELED_TYPES.contains(&kind.as_str()) { rep.new_keys.insert(kind.clone(), fresh); }
        }
        rep
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reports_only_the_unknown() {
        let mut s = Survey::default();
        s.add(&Record::parse(br#"{"type":"user","uuid":"u","version":"2.1.0","message":{"content":[{"type":"text","text":"x"},{"type":"hologram"}]}}"#, 0));
        s.add(&Record::parse(br#"{"type":"brand-new","sessionId":"s"}"#, 0));
        s.add(&Record::parse(br#"{"type":"attachment","attachment":{"type":"environment"}}"#, 0));
        s.add(&Record::parse(br#"{"type":"assistant","novelKey":1,"message":{"content":[]}}"#, 0));
        s.add(&Record::parse(b"garbage", 0));
        let r = s.report();
        assert_eq!(r.new_types, ["brand-new"]);
        assert_eq!(r.new_blocks, ["hologram"]);
        assert!(r.new_attachments.is_empty());
        assert_eq!(r.new_keys["assistant"], ["novelKey"]);
        assert!(!r.new_keys.contains_key("user"));
        assert_eq!(s.invalid_lines, 1);
        assert_eq!(r.versions, ["2.1.0"]);
    }
}
