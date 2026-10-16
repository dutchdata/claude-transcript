//! Reading JSONL files: complete lines with byte offsets, and the identity a tailer needs to resume safely.

use crate::record::Record;
use crate::util::sha256_hex;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

/// Iterator over complete lines (ending in `\n`) from a byte offset. A trailing partial line, as written by a
/// live Claude Code session, is not yielded; `offset()` after iteration says where to resume.
pub struct Lines {
    r: BufReader<File>,
    offset: u64,
    buf: Vec<u8>,
}

impl Lines {
    pub fn open(path: &Path, from: u64) -> std::io::Result<Lines> {
        let mut r = BufReader::with_capacity(1 << 20, File::open(path)?);
        r.seek(SeekFrom::Start(from))?;
        Ok(Lines { r, offset: from, buf: Vec::with_capacity(1 << 16) })
    }
    /// Byte offset just after the last complete line consumed.
    pub fn offset(&self) -> u64 { self.offset }

    /// Next complete line as `(offset, bytes without newline)`. `None` at EOF or at a partial last line.
    pub fn next_line(&mut self) -> std::io::Result<Option<(u64, &[u8])>> {
        self.buf.clear();
        let n = self.r.read_until(b'\n', &mut self.buf)?;
        if n == 0 || self.buf.last() != Some(&b'\n') {
            return Ok(None);
        }
        let off = self.offset;
        self.offset += n as u64;
        let end = if self.buf.len() >= 2 && self.buf[self.buf.len() - 2] == b'\r' { n - 2 } else { n - 1 };
        Ok(Some((off, &self.buf[..end])))
    }

    /// Parse the rest of the file into records.
    pub fn records(&mut self) -> std::io::Result<Vec<Record>> {
        let mut out = vec![];
        while let Some((off, line)) = self.next_line()? {
            out.push(Record::parse(line, off));
        }
        Ok(out)
    }
}

/// Read a whole file into records (lines only; a partial last line is skipped).
pub fn read_records(path: &Path) -> std::io::Result<Vec<Record>> {
    Lines::open(path, 0)?.records()
}

/// What a tailer stores per file to decide between "resume at offset" and "start over".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileId {
    pub inode: u64,
    pub size: u64,
    pub mtime: i64,
    /// SHA-256 of the first `head_len` bytes (at most 4 KB): the same path can become a different file
    /// (a rotated log, a transcript copied in from another machine).
    pub head_hash: String,
    pub head_len: u64,
}

fn head(path: &Path, n: u64) -> std::io::Result<String> {
    let mut f = File::open(path)?;
    let mut buf = vec![0u8; n.min(4096) as usize];
    let mut got = 0;
    while got < buf.len() {
        let k = f.read(&mut buf[got..])?;
        if k == 0 { break; }
        got += k;
    }
    Ok(sha256_hex(&buf[..got]))
}

impl FileId {
    pub fn of(path: &Path) -> std::io::Result<FileId> {
        let md = std::fs::metadata(path)?;
        #[cfg(unix)]
        let (inode, mtime) = { use std::os::unix::fs::MetadataExt; (md.ino(), md.mtime()) };
        #[cfg(not(unix))]
        let (inode, mtime) = (0u64, md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0));
        let head_len = md.len().min(4096);
        Ok(FileId { inode, size: md.len(), mtime, head_hash: head(path, head_len)?, head_len })
    }

    /// Decide where to read from now, given what was stored last time: `(current id, offset)`, where the offset
    /// is `None` when nothing changed, the stored offset when the same file grew, or 0 when it is a different file.
    pub fn resume(path: &Path, prev: Option<(&FileId, u64)>) -> std::io::Result<(FileId, Option<u64>)> {
        let now = FileId::of(path)?;
        let from = match prev {
            Some((p, _)) if *p == now => None,
            Some((p, off)) if p.inode == now.inode && now.size >= off && off >= p.head_len && head(path, p.head_len)? == p.head_hash => Some(off),
            _ => Some(0),
        };
        Ok((now, from))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn partial_last_line_is_not_consumed() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.jsonl");
        let mut f = File::create(&p).unwrap();
        write!(f, "{{\"type\":\"a\"}}\n{{\"type\":\"b\"}}\r\n{{\"type\":\"c\"").unwrap();
        let mut l = Lines::open(&p, 0).unwrap();
        let recs = l.records().unwrap();
        assert_eq!(recs.iter().map(|r| r.kind.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(recs[1].offset, 13);
        let resume = l.offset();
        writeln!(f, "}}").unwrap();
        let mut l = Lines::open(&p, resume).unwrap();
        assert_eq!(l.records().unwrap()[0].kind, "c");
    }
    #[test]
    fn file_id_resume_policy() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.jsonl");
        std::fs::write(&p, "{\"type\":\"a\"}\n").unwrap(); // 13 bytes
        let (id1, from) = FileId::resume(&p, None).unwrap();
        assert_eq!(from, Some(0));
        assert_eq!(FileId::resume(&p, Some((&id1, 13))).unwrap().1, None, "unchanged file");
        std::fs::OpenOptions::new().append(true).open(&p).unwrap().write_all(b"{\"type\":\"b\"}\n").unwrap();
        let (id2, from) = FileId::resume(&p, Some((&id1, 13))).unwrap();
        assert_eq!(from, Some(13), "same file grown: resume");
        std::fs::write(&p, "{\"type\":\"zzz\"}\n{\"type\":\"zzz\"}\n").unwrap();
        assert_eq!(FileId::resume(&p, Some((&id2, 26))).unwrap().1, Some(0), "rewritten: start over");
        std::fs::write(&p, "{\"type\":\"zzz\"}\n").unwrap();
        assert_eq!(FileId::resume(&p, Some((&id2, 26))).unwrap().1, Some(0), "shorter than the offset: start over");
    }
}
