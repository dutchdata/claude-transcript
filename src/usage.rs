//! Token and cost totals from the event stream.

use crate::events::Event;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Totals {
    pub messages: u64,
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub thinking: i64,
    pub tool_calls: u64,
    pub prompts: u64,
}

impl Totals {
    pub fn add(&mut self, e: &Event) {
        match e.kind {
            "usage" => {
                self.messages += 1;
                self.input += e.tok_in.unwrap_or(0);
                self.output += e.tok_out.unwrap_or(0);
                self.cache_read += e.tok_cache_read.unwrap_or(0);
                self.cache_write += e.tok_cache_write.unwrap_or(0);
                self.thinking += e.tok_reasoning.unwrap_or(0);
            }
            "tool_call" => self.tool_calls += 1,
            "prompt" => self.prompts += 1,
            _ => {}
        }
    }
    /// Share of context served from cache: cache_read / (input + cache_read + cache_write).
    pub fn cache_hit_ratio(&self) -> f64 {
        let d = self.input + self.cache_read + self.cache_write;
        if d == 0 { 0.0 } else { self.cache_read as f64 / d as f64 }
    }
}

/// Group key for totals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum By { Model, Day, Session, Tool }

/// Totals per group, with usage events deduplicated by API message id (several lines share one message).
pub fn totals(events: &[Event], by: By) -> BTreeMap<String, Totals> {
    let mut seen = std::collections::HashSet::new();
    let mut out: BTreeMap<String, Totals> = BTreeMap::new();
    for e in events {
        if e.kind == "usage" && !seen.insert(e.event_id.clone()) { continue; }
        let key = match by {
            By::Model => e.model.clone().unwrap_or_else(|| "-".into()),
            By::Day => e.ts.format("%Y-%m-%d").to_string(),
            By::Session => e.session_id.clone().unwrap_or_else(|| "-".into()),
            By::Tool => { if e.kind != "tool_call" { continue; } e.tool.clone().unwrap_or_else(|| "-".into()) }
        };
        out.entry(key).or_default().add(e);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    fn ev(kind: &'static str, model: &str, id: &str, tin: i64) -> Event { Event { event_id: id.into(), ts: Utc::now(), kind, model: Some(model.into()), tok_in: Some(tin), tok_cache_read: Some(10), ..Default::default() } }
    #[test]
    fn dedupes_usage_by_id() {
        let es = vec![ev("usage", "a", "x", 5), ev("usage", "a", "x", 5), ev("usage", "b", "y", 7), ev("tool_call", "a", "t", 0)];
        let t = totals(&es, By::Model);
        assert_eq!(t["a"].messages, 1);
        assert_eq!(t["a"].input, 5);
        assert_eq!(t["a"].tool_calls, 1);
        assert_eq!(t["b"].input, 7);
        assert!((t["b"].cache_hit_ratio() - 10.0 / 17.0).abs() < 1e-9);
    }
}
