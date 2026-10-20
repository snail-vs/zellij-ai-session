//! Session file format shared by Pi and its fork OMP (oh-my-pi).
//!
//! Both tools keep one JSONL file per session below
//! `<agent-dir>/sessions/<mangled-cwd>/<timestamp>_<uuid>.jsonl`. A `session`
//! entry carries the session id, creation time and working directory, and
//! `message` entries carry the transcript. OMP additionally writes a `title`
//! entry, always on the first line, holding the auto-generated session title.
//!
//! Pi puts the `session` entry on the first line, OMP puts its `title` entry
//! there and moves `session` to the second line, so the header is searched for
//! instead of being read positionally.

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;
use zellij_ai_session_core::PreviewMessage;

use crate::adapters::{clean_title, first_text, iso_to_ms, preview_message};

/// Bytes read from the end of a session file to find its last activity.
const TAIL_BYTES: u64 = 64 * 1024;

/// Metadata of a session file.
#[derive(Debug, Clone)]
pub struct SessionFile {
    /// Session uuid taken from the `session` entry.
    pub id: String,
    /// Working directory the session was started in.
    pub cwd: PathBuf,
    /// Creation time from the `session` entry.
    pub created_at_ms: Option<i64>,
    /// Last activity recorded in the file.
    pub updated_at_ms: Option<i64>,
    /// Title, when the file records one.
    pub title: Option<String>,
}

/// Read the header of a session file.
///
/// Returns `None` when the file carries no `session` entry, which is the case
/// for the transcript files OMP writes next to a session for each subagent.
pub fn read_header(path: &Path) -> Result<Option<SessionFile>> {
    let file = File::open(path).with_context(|| format!("open session file {}", path.display()))?;
    let reader = BufReader::new(file);

    let mut id = None;
    let mut cwd = None;
    let mut created_at_ms = None;
    let mut title = None;
    let mut explicit_title = false;

    for line in reader.lines() {
        let line = line.with_context(|| format!("read session file {}", path.display()))?;
        if !mentions_entry(&line) {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match entry.get("type").and_then(Value::as_str) {
            Some("session") => {
                id = entry.get("id").and_then(Value::as_str).map(str::to_owned);
                cwd = entry.get("cwd").and_then(Value::as_str).map(PathBuf::from);
                created_at_ms = entry
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .and_then(iso_to_ms);
            }
            Some("title") => {
                if let Some(value) = entry
                    .get("title")
                    .and_then(Value::as_str)
                    .map(clean_title)
                    .filter(|title| !title.is_empty())
                {
                    title = Some(value);
                    explicit_title = true;
                }
            }
            Some("session_info") => {
                if let Some(value) = entry
                    .get("name")
                    .and_then(Value::as_str)
                    .map(clean_title)
                    .filter(|title| !title.is_empty())
                {
                    title = Some(value);
                    explicit_title = true;
                }
            }
            // Fall back to the first user prompt, the way Pi titles a
            // session that has no explicit name.
            Some("message") if title.is_none() => {
                title = first_user_text(&entry);
            }
            _ => {}
        }
        if id.is_some() && explicit_title {
            break;
        }
    }

    let Some(id) = id else {
        return Ok(None);
    };
    let Some(cwd) = cwd else {
        return Ok(None);
    };
    Ok(Some(SessionFile {
        id,
        cwd,
        created_at_ms,
        updated_at_ms: tail_timestamp(path),
        title,
    }))
}

/// Read the transcript of a session file as preview messages.
pub fn read_messages(path: &Path) -> Result<Vec<PreviewMessage>> {
    let file = File::open(path).with_context(|| format!("open session file {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut messages = Vec::new();
    for line in reader.lines() {
        let line = line.with_context(|| format!("read session file {}", path.display()))?;
        if !line.contains("\"type\":\"message\"") {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if entry.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let Some(message) = entry.get("message") else {
            continue;
        };
        let Some(role) = message.get("role").and_then(Value::as_str) else {
            continue;
        };
        let Some(content) = message.get("content") else {
            continue;
        };
        if let Some(message) = preview_message(role, content) {
            messages.push(message);
        }
    }
    Ok(messages)
}

/// Locate the session file with the given id below `root`.
///
/// OMP names session files `<timestamp>_<uuid>.jsonl`, so the id is normally
/// found from the file name alone; the id inside the file is only compared
/// when the name does not follow that convention.
pub fn find_session_file(root: &Path, id: &str) -> Option<PathBuf> {
    let suffix = format!("_{id}");
    let files = session_files(root);
    if let Some(path) = files.iter().find(|path| {
        path.file_stem()
            .is_some_and(|stem| stem.to_string_lossy().ends_with(&suffix))
    }) {
        return Some(path.clone());
    }
    files
        .iter()
        .find(|path| {
            read_header(path)
                .ok()
                .flatten()
                .is_some_and(|header| header.id == id)
        })
        .cloned()
}

/// All session files below `root`, one level of project directories deep.
///
/// Subagent transcripts and tool logs live in a directory next to the session
/// file, so only the files directly inside a project directory are sessions.
pub fn session_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(projects) = std::fs::read_dir(root) else {
        return files;
    };
    for project in projects.flatten() {
        let path = project.path();
        if !path.is_dir() {
            if path.extension().and_then(|extension| extension.to_str()) == Some("jsonl") {
                files.push(path);
            }
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) == Some("jsonl") {
                files.push(path);
            }
        }
    }
    files
}

/// Whether a line can hold an entry the header is interested in.
fn mentions_entry(line: &str) -> bool {
    line.contains("\"type\":\"session\"")
        || line.contains("\"type\":\"title\"")
        || line.contains("\"type\":\"session_info\"")
        || line.contains("\"type\":\"message\"")
}

/// Plain text of the first user prompt in a message entry.
fn first_user_text(entry: &Value) -> Option<String> {
    let message = entry.get("message")?;
    if message.get("role").and_then(Value::as_str) != Some("user") {
        return None;
    }
    let text = first_text(message.get("content")?)?.trim();
    if text.is_empty() {
        return None;
    }
    Some(clean_title(text))
}

/// Timestamp of the last entry that recorded one.
///
/// The tail of the file is read instead of the whole file, because a single
/// session can hold tens of megabytes of tool output.
fn tail_timestamp(path: &Path) -> Option<i64> {
    let mut file = File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    if size == 0 {
        return None;
    }
    let offset = size.saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).ok()?;

    let mut lines: Vec<&[u8]> = buffer.split(|byte| *byte == b'\n').collect();
    if offset > 0 && !lines.is_empty() {
        // The first line was cut in half by the seek.
        lines.remove(0);
    }
    for line in lines.iter().rev() {
        if line.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if let Some(timestamp) = entry_timestamp(&entry) {
            return Some(timestamp);
        }
    }
    None
}

/// Timestamp of a session entry, in epoch milliseconds.
fn entry_timestamp(entry: &Value) -> Option<i64> {
    entry
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(iso_to_ms)
        .or_else(|| {
            entry
                .get("time")
                .and_then(|time| time.get("created"))
                .and_then(Value::as_i64)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("pi-format-{name}-{}.jsonl", std::process::id()))
    }

    fn write(path: &Path, contents: &str) {
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn reads_a_pi_session_with_the_session_entry_first() {
        let path = temp_path("pi");
        write(
            &path,
            "{\"type\":\"session\",\"version\":3,\"id\":\"pi-id\",\"timestamp\":\"2026-01-02T03:04:05.000Z\",\"cwd\":\"/tmp/pi\"}\n\
             {\"type\":\"message\",\"timestamp\":\"2026-01-02T03:05:00.000Z\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"hello pi\"}]}}\n",
        );
        let header = read_header(&path).unwrap().unwrap();
        assert_eq!(header.id, "pi-id");
        assert_eq!(header.cwd, PathBuf::from("/tmp/pi"));
        assert_eq!(
            header.created_at_ms,
            Some(iso_to_ms("2026-01-02T03:04:05.000Z").unwrap())
        );
        // The user prompt is used as the title.
        assert_eq!(header.title.as_deref(), Some("hello pi"));
        // The last recorded activity comes from the tail of the file.
        assert_eq!(
            header.updated_at_ms,
            Some(iso_to_ms("2026-01-02T03:05:00.000Z").unwrap())
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn reads_an_omp_session_with_the_title_entry_first() {
        let path = temp_path("omp");
        write(
            &path,
            "{\"type\":\"title\",\"v\":1,\"title\":\"OMP title\",\"source\":\"auto\"}\n\
             {\"type\":\"session\",\"version\":3,\"id\":\"omp-id\",\"timestamp\":\"2026-01-02T03:04:05.000Z\",\"cwd\":\"/tmp/omp\"}\n\
             {\"type\":\"message\",\"timestamp\":\"2026-01-02T03:06:00.000Z\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"done\"}]}}\n",
        );
        let header = read_header(&path).unwrap().unwrap();
        assert_eq!(header.id, "omp-id");
        assert_eq!(header.cwd, PathBuf::from("/tmp/omp"));
        assert_eq!(header.title.as_deref(), Some("OMP title"));
        assert_eq!(
            header.updated_at_ms,
            Some(iso_to_ms("2026-01-02T03:06:00.000Z").unwrap())
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn ignores_files_without_a_session_entry() {
        let path = temp_path("subagent");
        write(
            &path,
            "{\"type\":\"title\",\"v\":1,\"title\":\"subagent\"}\n\
             {\"type\":\"message\",\"timestamp\":\"2026-01-02T03:06:00.000Z\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
        );
        assert!(read_header(&path).unwrap().is_none());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn finds_a_session_file_by_id() {
        let root = temp_path("root");
        let _ = std::fs::remove_dir_all(&root);
        let project = root.join("--tmp-demo--");
        std::fs::create_dir_all(&project).unwrap();
        let path = project.join("2026-01-02T03-04-05-000Z_omp-id.jsonl");
        write(
            &path,
            "{\"type\":\"title\",\"v\":1,\"title\":\"demo\"}\n\
             {\"type\":\"session\",\"version\":3,\"id\":\"omp-id\",\"timestamp\":\"2026-01-02T03:04:05.000Z\",\"cwd\":\"/tmp/demo\"}\n",
        );
        // A subagent transcript next to the session must not be picked up.
        let subagent_dir = project.join("2026-01-02T03-04-05-000Z_omp-id");
        std::fs::create_dir_all(&subagent_dir).unwrap();
        write(
            &subagent_dir.join("Planner.jsonl"),
            "{\"type\":\"title\",\"v\":1,\"title\":\"sub\"}\n",
        );
        assert_eq!(find_session_file(&root, "omp-id"), Some(path));
        assert_eq!(find_session_file(&root, "other"), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reads_only_user_and_assistant_messages() {
        let path = temp_path("messages");
        write(
            &path,
            "{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"question\"}]}}\n\
             {\"type\":\"message\",\"message\":{\"role\":\"toolResult\",\"content\":[{\"type\":\"text\",\"text\":\"tool output\"}]}}\n\
             {\"type\":\"message\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"answer\"}]}}\n",
        );
        let messages = read_messages(&path).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[1].text, "answer");
        std::fs::remove_file(path).unwrap();
    }
}
