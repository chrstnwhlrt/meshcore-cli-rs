//! History files in the format of `prompt_toolkit`'s `FileHistory`.
//!
//! The reference stores its history like this:
//!
//! ```text
//!
//! # 2026-11-24 14:38:23.123456
//! +first line of the entry
//! +second line of a multi-line entry
//! ```
//!
//! Reading and writing the same format lets the Python and the Rust CLI
//! share `~/.config/meshcore/history`.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// A history file in `prompt_toolkit` format.
#[derive(Debug, Clone)]
pub struct FileHistory {
    path: PathBuf,
    last: Option<String>,
}

impl FileHistory {
    /// Opens (but does not create) a history file.
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path, last: None }
    }

    /// Loads all entries, oldest first.
    pub fn load(&mut self) -> Vec<String> {
        let entries = std::fs::read(&self.path)
            .map(|bytes| parse(&String::from_utf8_lossy(&bytes)))
            .unwrap_or_default();
        self.last = entries.last().cloned();
        entries
    }

    /// Appends an entry unless it repeats the previous one.
    pub fn append(&mut self, entry: &str) {
        if entry.is_empty() || self.last.as_deref() == Some(entry) {
            return;
        }
        self.last = Some(entry.to_string());
        let _ = append_entry(&self.path, entry);
    }
}

/// Parses the file content into entries, oldest first.
#[must_use]
pub fn parse(content: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut lines: Vec<&str> = Vec::new();
    for line in content.split_inclusive('\n') {
        if let Some(rest) = line.strip_prefix('+') {
            lines.push(rest);
        } else {
            push_entry(&mut entries, &lines);
            lines.clear();
        }
    }
    push_entry(&mut entries, &lines);
    entries
}

/// Joins collected lines and drops the trailing newline.
fn push_entry(entries: &mut Vec<String>, lines: &[&str]) {
    if lines.is_empty() {
        return;
    }
    let mut joined: String = lines.concat();
    joined.pop();
    entries.push(joined);
}

/// Formats one entry the way `prompt_toolkit` stores it.
#[must_use]
pub fn format_entry(entry: &str, timestamp: &str) -> String {
    let mut out = format!("\n# {timestamp}\n");
    for line in entry.split('\n') {
        out.push('+');
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// The local time like Python's `str(datetime.datetime.now())`, which
/// `prompt_toolkit` writes: microseconds unless there are none.
fn now_text() -> String {
    let now = crate::util::time::local_zone().to_datetime(jiff::Timestamp::now());
    let text = now.strftime("%Y-%m-%d %H:%M:%S").to_string();
    match now.subsec_nanosecond() / 1000 {
        0 => text,
        micros => format!("{text}.{micros:06}"),
    }
}

fn append_entry(path: &Path, entry: &str) -> std::io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(format_entry(entry, &now_text()).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_prompt_toolkit_format() {
        let content = "\n# 2026-01-01 10:00:00.000001\n+ver\n\n# 2026-01-01 10:00:01.000001\n+line one\n+line two\n";
        assert_eq!(
            parse(content),
            vec!["ver".to_string(), "line one\nline two".to_string()]
        );
    }

    #[test]
    fn roundtrip() {
        let mut content = String::new();
        content.push_str(&format_entry("msg Alice hi", "2026-01-01 10:00:00.000001"));
        content.push_str(&format_entry("a\nb", "2026-01-01 10:00:00.000002"));
        assert_eq!(
            parse(&content),
            vec!["msg Alice hi".to_string(), "a\nb".to_string()]
        );
    }

    #[test]
    fn tolerates_missing_final_newline() {
        assert_eq!(parse("\n# x\n+abc"), vec!["ab".to_string()]);
    }

    #[test]
    fn append_skips_duplicates() {
        let dir = std::env::temp_dir().join(format!("mccli-hist-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history");
        let _ = std::fs::remove_file(&path);
        let mut h = FileHistory::new(path.clone());
        h.append("one");
        h.append("one");
        h.append("two");
        let mut h2 = FileHistory::new(path.clone());
        assert_eq!(h2.load(), vec!["one".to_string(), "two".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
