//! Find transcripts under a Claude Code projects root (`~/.claude/projects` by default).

use serde_json::Value;
use std::path::{Path, PathBuf};

/// One transcript file: a session, or a subagent/workflow agent of a session.
#[derive(Debug, Clone)]
pub struct TranscriptFile {
    pub path: PathBuf,
    /// Project directory name (Claude Code's slug of the working directory, e.g. `-Users-you-src-x`).
    pub project: String,
    /// Session id from the file name (sessions) or from the enclosing `<session>/subagents/` directory (agents).
    pub session_id: String,
    pub subagent: Option<Subagent>,
}

#[derive(Debug, Clone)]
pub struct Subagent {
    pub agent_id: String,
    /// Workflow run id when the agent ran inside `workflows/<wf>/`.
    pub workflow: Option<String>,
    /// Contents of `agent-<id>.meta.json` (agentType, description, model, …), if present.
    pub meta: Value,
}

impl TranscriptFile {
    pub fn agent_type(&self) -> Option<&str> { self.subagent.as_ref()?.meta.get("agentType")?.as_str() }
    pub fn description(&self) -> Option<&str> { self.subagent.as_ref()?.meta.get("description")?.as_str() }
    pub fn model(&self) -> Option<&str> { self.subagent.as_ref()?.meta.get("model")?.as_str() }
}

/// `~/.claude/projects`, honoring `CLAUDE_CONFIG_DIR`.
pub fn default_root() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("CLAUDE_CONFIG_DIR") {
        return Some(PathBuf::from(d).join("projects"));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".claude").join("projects"))
}

/// Every `.jsonl` under the root, classified. Sorted by path.
pub fn find(root: &Path) -> Vec<TranscriptFile> {
    let mut out: Vec<TranscriptFile> = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file() && e.path().extension().and_then(|x| x.to_str()) == Some("jsonl"))
        .filter_map(|e| classify(root, e.path()))
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Classify one path relative to the root. `None` when it is not under a project directory.
pub fn classify(root: &Path, path: &Path) -> Option<TranscriptFile> {
    let rel = path.strip_prefix(root).ok()?;
    let mut comps: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
    if comps.len() < 2 {
        return None;
    }
    let project = comps.remove(0);
    let stem = path.file_stem()?.to_string_lossy().to_string();
    if comps.len() == 1 {
        return Some(TranscriptFile { path: path.to_path_buf(), project, session_id: stem, subagent: None });
    }
    // <session>/subagents/agent-<id>.jsonl or <session>/subagents/workflows/<wf>/agent-<id>.jsonl
    if comps.get(1).map(String::as_str) != Some("subagents") {
        return None;
    }
    let session_id = comps[0].clone();
    let workflow = (comps.get(2).map(String::as_str) == Some("workflows")).then(|| comps.get(3).cloned()).flatten();
    let agent_id = stem.strip_prefix("agent-").unwrap_or(&stem).to_string();
    let meta = std::fs::read(path.with_extension("meta.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
    Some(TranscriptFile { path: path.to_path_buf(), project, session_id, subagent: Some(Subagent { agent_id, workflow, meta }) })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classifies_sessions_and_agents() {
        let root = Path::new("/r");
        let s = classify(root, Path::new("/r/-Users-j/abc.jsonl")).unwrap();
        assert_eq!((s.project.as_str(), s.session_id.as_str()), ("-Users-j", "abc"));
        assert!(s.subagent.is_none());
        let a = classify(root, Path::new("/r/-Users-j/abc/subagents/agent-a1.jsonl")).unwrap();
        assert_eq!(a.session_id, "abc");
        assert_eq!(a.subagent.as_ref().unwrap().agent_id, "a1");
        let w = classify(root, Path::new("/r/-Users-j/abc/subagents/workflows/wf_1/agent-a2.jsonl")).unwrap();
        assert_eq!(w.subagent.as_ref().unwrap().workflow.as_deref(), Some("wf_1"));
        assert!(classify(root, Path::new("/r/-Users-j/abc/other/x.jsonl")).is_none());
        assert!(classify(root, Path::new("/r/top.jsonl")).is_none());
    }
}
