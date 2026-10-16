//! cct: Claude Code transcripts from the command line.
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand, ValueEnum};
use claude_transcript::discover::{self, TranscriptFile};
use claude_transcript::events::{events as to_events, Ctx, Event};
use claude_transcript::reader::{read_records, Lines};
use claude_transcript::thread::{AgentInput, Thread};
use claude_transcript::usage::{totals, By};
use claude_transcript::{drift, Record};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Parser)]
#[command(name = "cct", version, about = "Parse Claude Code session transcripts (~/.claude/projects/**/*.jsonl)")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Sessions and subagents under a projects root (default ~/.claude/projects)
    Sessions { root: Option<PathBuf>, #[arg(long)] json: bool },
    /// Flat event stream (prompt, tool_call, tool_result, usage, compact, …) for files or directories
    Events { paths: Vec<PathBuf>, #[arg(long)] json: bool, #[arg(long)] kind: Option<String>, #[arg(long)] since: Option<String>, #[arg(long, default_value_t = 400)] excerpt: usize },
    /// The session as a thread: turns, tool calls with their results, reactions, nested subagents. Lossless.
    Render { session: PathBuf, #[arg(long, default_value_t = 0)] max: usize, #[arg(long)] json: bool },
    /// Token totals by model, day, session or tool (usage deduplicated per API message)
    Usage { paths: Vec<PathBuf>, #[arg(long, value_enum, default_value_t = ByArg::Model)] by: ByArg, #[arg(long)] since: Option<String>, #[arg(long)] json: bool },
    /// What the corpus contains that this crate does not know yet (run after a Claude Code update)
    Drift { root: Option<PathBuf>, #[arg(long)] json: bool, #[arg(long)] full: bool },
    /// Size, record and parse-speed numbers for a corpus
    Stats { root: Option<PathBuf> },
    /// Follow a live transcript and print events as they are written
    Tail { file: PathBuf, #[arg(long, default_value_t = 0)] from: u64, #[arg(long)] json: bool },
}

#[derive(Clone, Copy, ValueEnum)]
enum ByArg { Model, Day, Session, Tool }

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Sessions { root, json } => {
            let root = root_or_default(root)?;
            let files = discover::find(&root);
            if json { println!("{}", serde_json::to_string_pretty(&files.iter().map(|f| serde_json::json!({"path": f.path, "project": f.project, "session_id": f.session_id, "agent_id": f.subagent.as_ref().map(|s| &s.agent_id), "agent_type": f.agent_type(), "description": f.description(), "model": f.model()})).collect::<Vec<_>>())?); return Ok(()); }
            let mut by: BTreeMap<String, (usize, usize)> = BTreeMap::new();
            for f in &files { let e = by.entry(f.project.clone()).or_default(); if f.subagent.is_some() { e.1 += 1 } else { e.0 += 1 } }
            for (p, (s, a)) in &by { println!("{s:5} sessions {a:5} agents  {p}"); }
            println!("{} files in {}", files.len(), root.display());
        }
        Cmd::Events { paths, json, kind, since, excerpt } => {
            let since = parse_since(since.as_deref())?;
            let mut out = std::io::BufWriter::new(std::io::stdout().lock());
            for f in expand(paths)? {
                for e in file_events(&f, excerpt)? {
                    if kind.as_deref().is_some_and(|k| k != e.kind) { continue; }
                    if since.is_some_and(|s| e.ts < s) { continue; }
                    if json { writeln!(out, "{}", serde_json::to_string(&e)?)?; } else { writeln!(out, "{}", fmt_line(&e))?; }
                }
            }
        }
        Cmd::Render { session, max, json } => {
            let recs = read_records(&session).with_context(|| session.display().to_string())?;
            let agents = agents_beside(&session)?;
            let th = Thread::build_with_agents(&recs, agents);
            if json { println!("{}", serde_json::to_string_pretty(&th)?); } else { print!("{}", th.render(max)); }
        }
        Cmd::Usage { paths, by, since, json } => {
            let since = parse_since(since.as_deref())?;
            let mut all = vec![];
            for f in expand(paths)? { all.extend(file_events(&f, 0)?.into_iter().filter(|e| !since.is_some_and(|s| e.ts < s))); }
            let t = totals(&all, match by { ByArg::Model => By::Model, ByArg::Day => By::Day, ByArg::Session => By::Session, ByArg::Tool => By::Tool });
            if json { println!("{}", serde_json::to_string_pretty(&t)?); return Ok(()); }
            if matches!(by, ByArg::Tool) {
                for (k, v) in &t { println!("{:8} {k}", v.tool_calls); }
                return Ok(());
            }
            println!("{:<34} {:>7} {:>12} {:>11} {:>13} {:>12} {:>9} {:>6}", "group", "msgs", "input", "output", "cache_read", "cache_write", "thinking", "hit");
            for (k, v) in t.iter().filter(|(_, v)| v.messages > 0 || v.tool_calls > 0) { println!("{:<34} {:>7} {:>12} {:>11} {:>13} {:>12} {:>9} {:>5.0}%", trunc(k, 34), v.messages, v.input, v.output, v.cache_read, v.cache_write, v.thinking, v.cache_hit_ratio() * 100.0); }
        }
        Cmd::Drift { root, json, full } => {
            let root = root_or_default(root)?;
            let mut s = drift::Survey::default();
            for f in discover::find(&root) { s.files += 1; for r in read_records(&f.path)? { s.add(&r); } }
            let rep = s.report();
            if json { println!("{}", serde_json::to_string_pretty(&if full { serde_json::to_value(&s)? } else { serde_json::to_value(&rep)? })?); return Ok(()); }
            println!("{} files, {} records, {} invalid lines, versions {:?}", s.files, s.records, s.invalid_lines, rep.versions);
            let none = rep.new_types.is_empty() && rep.new_keys.is_empty() && rep.new_blocks.is_empty() && rep.new_attachments.is_empty() && rep.new_system_subtypes.is_empty();
            if none { println!("no drift: everything in this corpus is known to claude-transcript {}", env!("CARGO_PKG_VERSION")); }
            if !rep.new_types.is_empty() { println!("new record types: {}", rep.new_types.join(", ")); }
            if !rep.new_blocks.is_empty() { println!("new content block types: {}", rep.new_blocks.join(", ")); }
            if !rep.new_attachments.is_empty() { println!("new attachment types: {}", rep.new_attachments.join(", ")); }
            if !rep.new_system_subtypes.is_empty() { println!("new system subtypes: {}", rep.new_system_subtypes.join(", ")); }
            for (k, v) in &rep.new_keys { println!("new keys on {k}: {}", v.join(", ")); }
            if full {
                println!("\nrecord types:"); for (k, v) in &s.types { println!("{v:8} {k}"); }
                println!("\ntools:"); for (k, v) in &s.tools { println!("{v:8} {k}"); }
                println!("\nmodels:"); for (k, v) in &s.models { println!("{v:8} {k}"); }
            }
        }
        Cmd::Stats { root } => {
            let root = root_or_default(root)?;
            let files = discover::find(&root);
            let t0 = Instant::now();
            let (mut bytes, mut records, mut events, mut sessions, mut agents) = (0u64, 0u64, 0u64, 0u64, 0u64);
            let mut kinds: BTreeMap<&'static str, u64> = BTreeMap::new();
            for f in &files {
                if f.subagent.is_some() { agents += 1 } else { sessions += 1 }
                let mut ctx = ctx_for(f, 0);
                let mut l = Lines::open(&f.path, 0)?;
                while let Some((off, line)) = l.next_line()? {
                    bytes += line.len() as u64 + 1; records += 1;
                    let r = Record::parse(line, off);
                    let (es, _) = to_events(&r, &mut ctx);
                    for e in es { events += 1; *kinds.entry(e.kind).or_default() += 1; }
                }
            }
            let secs = t0.elapsed().as_secs_f64();
            println!("{} sessions, {} agent transcripts, {:.1} MB, {} records → {} events in {:.2} s ({:.0} MB/s, {:.0}k records/s)", sessions, agents, bytes as f64 / 1e6, records, events, secs, bytes as f64 / 1e6 / secs, records as f64 / 1e3 / secs);
            for (k, v) in &kinds { println!("{v:10} {k}"); }
        }
        Cmd::Tail { file, from, json } => {
            let mut off = from;
            let mut ctx = Ctx::new(file.display().to_string());
            loop {
                let mut l = Lines::open(&file, off)?;
                while let Some((o, line)) = l.next_line()? {
                    let r = Record::parse(line, o);
                    let (es, _) = to_events(&r, &mut ctx);
                    for e in es { if json { println!("{}", serde_json::to_string(&e)?); } else { println!("{}", fmt_line(&e)); } }
                }
                off = l.offset();
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }
    }
    Ok(())
}

fn root_or_default(root: Option<PathBuf>) -> Result<PathBuf> {
    root.or_else(discover::default_root).context("no projects root: pass a path or set HOME / CLAUDE_CONFIG_DIR")
}

/// Files and directories → transcript files (a bare file outside a projects root is treated as a session).
fn expand(paths: Vec<PathBuf>) -> Result<Vec<TranscriptFile>> {
    let paths = if paths.is_empty() { vec![root_or_default(None)?] } else { paths };
    let mut out = vec![];
    for p in paths {
        if p.is_dir() {
            let found = discover::find(&p);
            if found.is_empty() {
                for e in walkdir::WalkDir::new(&p).into_iter().filter_map(|e| e.ok()).filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("jsonl")) { out.push(bare(e.path())); }
            } else { out.extend(found); }
        } else { out.push(bare(&p)); }
    }
    Ok(out)
}
fn bare(p: &Path) -> TranscriptFile {
    let root = discover::default_root();
    root.as_deref().and_then(|r| discover::classify(r, p)).unwrap_or_else(|| TranscriptFile { path: p.to_path_buf(), project: String::new(), session_id: p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(), subagent: None })
}
fn ctx_for(f: &TranscriptFile, excerpt: usize) -> Ctx {
    let mut c = match &f.subagent { Some(s) => Ctx::for_agent(f.path.display().to_string(), s.agent_id.clone(), s.meta.clone()), None => Ctx::new(f.path.display().to_string()) };
    if excerpt > 0 { c.excerpt_len = excerpt; }
    c
}
fn file_events(f: &TranscriptFile, excerpt: usize) -> Result<Vec<Event>> {
    let mut ctx = ctx_for(f, excerpt);
    let mut out = vec![];
    for r in read_records(&f.path).with_context(|| f.path.display().to_string())? { out.extend(to_events(&r, &mut ctx).0); }
    Ok(out)
}
/// `<session>/subagents/**/agent-*.jsonl` next to a session file.
fn agents_beside(session: &Path) -> Result<Vec<AgentInput>> {
    let dir = session.with_extension("").join("subagents");
    let mut out = vec![];
    if !dir.is_dir() { return Ok(out); }
    for e in walkdir::WalkDir::new(&dir).into_iter().filter_map(|e| e.ok()).filter(|e| e.file_type().is_file() && e.path().extension().and_then(|x| x.to_str()) == Some("jsonl")) {
        let stem = e.path().file_stem().unwrap().to_string_lossy().to_string();
        let meta = std::fs::read(e.path().with_extension("meta.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(serde_json::Value::Null);
        out.push(AgentInput { agent_id: stem.trim_start_matches("agent-").to_string(), meta, records: read_records(e.path())? });
    }
    out.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
    Ok(out)
}
fn parse_since(s: Option<&str>) -> Result<Option<DateTime<Utc>>> {
    let Some(s) = s else { return Ok(None) };
    if let Ok(d) = DateTime::parse_from_rfc3339(s) { return Ok(Some(d.with_timezone(&Utc))); }
    let d = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").with_context(|| format!("--since wants YYYY-MM-DD or RFC 3339, got {s}"))?;
    Ok(Some(d.and_hms_opt(0, 0, 0).unwrap().and_utc()))
}
fn fmt_line(e: &Event) -> String {
    let mut s = format!("{} {:<12} {}", e.ts.format("%Y-%m-%d %H:%M:%S"), e.kind, e.session_id.as_deref().map(|x| &x[..x.len().min(8)]).unwrap_or("-"));
    if let Some(a) = &e.agent_id { s.push_str(&format!(" agent:{}", &a[..a.len().min(8)])); }
    if let Some(t) = &e.tool { s.push_str(&format!(" {t}")); }
    if e.kind == "usage" { s.push_str(&format!(" {} in {} out {} cr {} cw {}", e.model.as_deref().unwrap_or(""), e.tok_in.unwrap_or(0), e.tok_out.unwrap_or(0), e.tok_cache_read.unwrap_or(0), e.tok_cache_write.unwrap_or(0))); }
    if let Some(t) = &e.text { s.push_str("  "); s.push_str(&t.replace('\n', " ")); }
    if e.is_error == Some(true) { s.push_str("  [error]"); }
    s
}
fn trunc(s: &str, n: usize) -> String { if s.chars().count() > n { s.chars().take(n - 1).collect::<String>() + "…" } else { s.to_string() } }
