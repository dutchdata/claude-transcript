//! Parse Claude Code session transcripts.
//!
//! Claude Code writes one JSON Lines file per session under `~/.claude/projects/<project-slug>/<session>.jsonl`,
//! plus one per subagent under `<session>/subagents/[workflows/<wf>/]agent-<id>.jsonl` (with an
//! `agent-<id>.meta.json` beside it). The format is undocumented and grows with every release; this crate
//! parses it losslessly (every record keeps its raw JSON), classifies what it knows, tolerates what it does not,
//! and tells you what changed (`drift`).
//!
//! Layers, from raw to cooked:
//! - [`reader`]: complete lines with byte offsets; resumable (`FileId`) so a tailer can pick up where it left off.
//! - [`record`]: one line → [`Record`] with a typed [`View`] (user, assistant, system, attachment, …).
//! - [`events`]: records → a flat, stable-id [`Event`] stream (prompts, tool calls, results, usage, compactions…).
//! - [`thread`]: records → a tree ([`thread::Thread`]) with turns, replies, nested subagents and reactions, and a
//!   lossless text render.
//! - [`usage`]: token and cost totals by model, day or session.
//! - [`drift`]: survey a corpus for record types, keys, block types and attachment types the crate has not seen.
//! - [`discover`]: find session and subagent files under a projects root.

pub mod discover;
pub mod drift;
pub mod events;
pub mod reader;
pub mod record;
pub mod thread;
pub mod usage;
pub mod util;

pub use events::{Ctx, Event, SessionSignal};
pub use reader::{FileId, Lines};
pub use record::{Block, Message, Record, Usage, View};
pub use thread::Thread;

/// Errors from reading files; JSON that does not parse is not an error (it becomes a `Record` with `View::Invalid`).
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self { Error::Io(e) }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self { Error::Io(e) => write!(f, "io: {e}") }
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

/// The version of the transcript format this crate was last verified against (Claude Code `version` field).
pub const KNOWN_CLAUDE_CODE_VERSION: &str = "2.1.301";
