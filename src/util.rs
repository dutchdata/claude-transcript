//! Small shared helpers: excerpts, stable ids, timestamps.

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Default excerpt length used by the event stream.
pub const EXCERPT: usize = 400;

/// First `n` chars with whitespace collapsed to single spaces; `…` appended when cut.
pub fn excerpt(s: &str, n: usize) -> String {
    let mut out = String::with_capacity(n.min(s.len()) + 3);
    let mut count = 0;
    let mut last_space = false;
    for c in s.chars() {
        let c = if c.is_whitespace() { ' ' } else { c };
        if c == ' ' {
            if last_space || count == 0 {
                continue;
            }
            last_space = true;
        } else {
            last_space = false;
        }
        if count == n {
            out.push('…');
            return out;
        }
        out.push(c);
        count += 1;
    }
    out.truncate(out.trim_end().len());
    out
}

/// 128-bit hex id from parts, separated by 0x1f. Deterministic: the same raw bytes always give the same id.
pub fn id(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update([0x1f]);
    }
    let d = h.finalize();
    d[..16].iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Timestamps appear as RFC 3339 strings; a few older records carry epoch seconds or milliseconds.
pub fn ts_value(v: &Value) -> Option<DateTime<Utc>> {
    match v {
        Value::String(s) => DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc)),
        Value::Number(n) => {
            let f = n.as_f64()?;
            if !f.is_finite() || f <= 0.0 {
                return None;
            }
            let secs = if f > 1e11 { f / 1000.0 } else { f };
            Utc.timestamp_millis_opt((secs * 1000.0) as i64).single()
        }
        _ => None,
    }
}

/// Fixed-width UTC (`2026-10-15T17:04:19.123Z`) so strings sort as times.
pub fn fmt_ts(t: &DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub(crate) fn s(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(str::to_string)
}
pub(crate) fn i(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(|x| x.as_i64())
}
pub(crate) fn b(v: &Value, key: &str) -> Option<bool> {
    v.get(key).and_then(|x| x.as_bool())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn excerpt_collapses_and_cuts() {
        assert_eq!(excerpt("  a \n\n b   c ", 10), "a b c");
        assert_eq!(excerpt("abcdef", 3), "abc…");
        assert_eq!(excerpt("", 3), "");
    }
    #[test]
    fn id_is_stable() {
        assert_eq!(id(&["a", "b"]), id(&["a", "b"]));
        assert_ne!(id(&["a", "b"]), id(&["ab"]));
        assert_eq!(id(&["x"]).len(), 32);
    }
    #[test]
    fn ts_forms() {
        assert!(ts_value(&Value::String("2026-10-15T17:04:19.123Z".into())).is_some());
        assert!(ts_value(&serde_json::json!(1792109059.0)).is_some());
        assert!(ts_value(&serde_json::json!(1792109059921u64)).is_some());
        assert!(ts_value(&Value::Null).is_none());
    }
}
