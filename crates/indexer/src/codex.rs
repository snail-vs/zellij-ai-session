use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;
use zellij_ai_session_core::{AgentKind, AiSession, CommandSpec};

use crate::adapters::AgentAdapter;

pub struct CodexAdapter {
    sessions_root: PathBuf,
    session_index: PathBuf,
}

impl CodexAdapter {
    pub fn new(sessions_root: PathBuf, session_index: PathBuf) -> Self {
        Self {
            sessions_root,
            session_index,
        }
    }

    fn parse_file(&self, path: &Path) -> Result<Option<(AiSession, bool)>> {
        let file =
            File::open(path).with_context(|| format!("open Codex session {}", path.display()))?;
        let reader = BufReader::new(file);
        let mut meta = None;
        let mut title = None;
        let mut last_timestamp = None;

        for line in reader.lines() {
            let line = line?;
            let value: Value = match serde_json::from_str(&line) {
                Ok(value) => value,
                Err(_) => continue,
            };
            let timestamp = value
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(parse_timestamp);
            last_timestamp = last_timestamp.max(timestamp);
            match value.get("type").and_then(Value::as_str) {
                Some("session_meta") => meta = Some(value),
                Some("response_item") => {
                    let payload = value.get("payload").unwrap_or(&value);
                    let role = payload.get("role").and_then(Value::as_str);
                    if matches!(role, Some("user" | "assistant")) {
                        let content = payload.get("content");
                        if title.is_none() && role == Some("user") {
                            title = content.and_then(first_text).and_then(clean_title);
                        }
                    }
                }
                _ => {}
            }
        }

        let meta = match meta {
            Some(meta) => meta,
            None => return Ok(None),
        };
        let payload = meta.get("payload").unwrap_or(&meta);
        let legacy_id = payload.get("session_id").and_then(Value::as_str).is_none();
        let session_id = payload
            .get("session_id")
            .and_then(Value::as_str)
            .or_else(|| payload.get("id").and_then(Value::as_str))
            .unwrap_or_default();
        if session_id.is_empty() {
            anyhow::bail!("session_meta has no native session ID");
        }
        let directory = payload
            .get("cwd")
            .and_then(Value::as_str)
            .map(PathBuf::from);
        let directory = match directory {
            Some(directory) => directory,
            None => return Ok(None),
        };
        let created_at_ms = payload
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_timestamp)
            .or_else(|| value_timestamp(&meta));
        let updated_at_ms = last_timestamp.or(created_at_ms);
        let title = title
            .unwrap_or_else(|| format!("Codex session {}", &session_id[..session_id.len().min(8)]));

        Ok(Some((
            AiSession::new(
                AgentKind::Codex,
                &session_id,
                title,
                directory,
                created_at_ms,
                updated_at_ms,
            ),
            legacy_id,
        )))
    }

    fn scan_pages(&self) -> Result<(Vec<AiSession>, Vec<String>)> {
        let mut pages: HashMap<String, AiSession> = HashMap::new();
        let mut warnings = Vec::new();
        if !self.sessions_root.exists() {
            return Ok((Vec::new(), warnings));
        }
        let thread_names = read_thread_names(&self.session_index)?;
        visit_jsonl(&self.sessions_root, &mut |path| {
            let (session, legacy_id) = match self.parse_file(path) {
                Ok(Some(parsed)) => parsed,
                Ok(None) => return Ok(()),
                Err(error) => {
                    warnings.push(format!("Codex {}: {error}", path.display()));
                    return Ok(());
                }
            };
            if legacy_id {
                warnings.push(format!(
                    "Codex {}: session_id absent; using legacy id",
                    path.display()
                ));
            }
            merge_page(&mut pages, session);
            Ok(())
        })?;
        for session in pages.values_mut() {
            if let Some(thread_name) = thread_names.get(&session.agent_session_id) {
                session.title = thread_name.clone();
            }
        }
        Ok((pages.into_values().collect(), warnings))
    }
}

impl AgentAdapter for CodexAdapter {
    fn name(&self) -> &'static str {
        "Codex"
    }
    fn agent(&self) -> AgentKind {
        AgentKind::Codex
    }

    fn list_sessions(&self) -> Result<Vec<AiSession>> {
        self.scan_pages().map(|(sessions, _)| sessions)
    }

    fn list_sessions_with_warnings(&self) -> Result<(Vec<AiSession>, Vec<String>)> {
        self.scan_pages()
    }

    fn resume_command(&self, session: &AiSession) -> Result<CommandSpec> {
        Ok(CommandSpec::new("codex", session.directory.clone())
            .with_args(["resume", session.agent_session_id.as_str()]))
    }
}

fn merge_page(pages: &mut HashMap<String, AiSession>, session: AiSession) {
    match pages.entry(session.agent_session_id.clone()) {
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(session);
        }
        std::collections::hash_map::Entry::Occupied(mut entry) => {
            let current = entry.get_mut();
            let earlier = matches!((session.created_at_ms, current.created_at_ms),
                (Some(left), Some(right)) if left < right);
            if earlier {
                current.directory = session.directory.clone();
                if !is_fallback_title(&session) {
                    current.title = session.title.clone();
                }
            } else if is_fallback_title(current) && !is_fallback_title(&session) {
                current.title = session.title.clone();
            }
            current.created_at_ms = match (current.created_at_ms, session.created_at_ms) {
                (Some(left), Some(right)) => Some(left.min(right)),
                (left, right) => left.or(right),
            };
            current.updated_at_ms = current.updated_at_ms.max(session.updated_at_ms);
        }
    }
}

fn is_fallback_title(session: &AiSession) -> bool {
    session.title.starts_with("Codex session ")
}

fn visit_jsonl(root: &Path, callback: &mut impl FnMut(&Path) -> Result<()>) -> Result<()> {
    for entry in std::fs::read_dir(root).with_context(|| format!("read {}", root.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            visit_jsonl(&path, callback)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            callback(&path)?;
        }
    }
    Ok(())
}

fn value_timestamp(value: &Value) -> Option<i64> {
    value
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_timestamp)
}

fn parse_timestamp(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.timestamp_millis())
}

fn read_thread_names(path: &Path) -> Result<HashMap<String, String>> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let file =
        File::open(path).with_context(|| format!("open Codex session index {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut names = HashMap::new();
    for line in reader.lines() {
        let line = line?;
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let Some(id) = value.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(thread_name) = value
            .get("thread_name")
            .and_then(Value::as_str)
            .and_then(clean_title)
        else {
            continue;
        };
        names.insert(id.to_string(), thread_name);
    }
    Ok(names)
}

fn clean_title(title: &str) -> Option<String> {
    let title = title.lines().next().unwrap_or(title).trim();
    if title.is_empty() {
        None
    } else {
        Some(title.chars().take(100).collect())
    }
}

fn first_text(value: &Value) -> Option<&str> {
    match value {
        Value::String(text) => Some(text),
        Value::Array(values) => values.iter().find_map(first_text),
        Value::Object(object) => object.get("text").and_then(Value::as_str),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn latest_codex_thread_name_wins() {
        let first =
            r#"{"id":"session-1","thread_name":"First name","updated_at":"2026-08-16T10:00:00Z"}"#;
        let second = r#"{"id":"session-1","thread_name":"Renamed session","updated_at":"2026-08-16T10:01:00Z"}"#;
        let mut names = HashMap::new();
        for line in [first, second] {
            let value: Value = serde_json::from_str(line).unwrap();
            let id = value.get("id").and_then(Value::as_str).unwrap();
            let title = value
                .get("thread_name")
                .and_then(Value::as_str)
                .and_then(clean_title)
                .unwrap();
            names.insert(id.to_string(), title);
        }
        assert_eq!(names.get("session-1"), Some(&"Renamed session".to_string()));
    }

    #[test]
    fn codex_title_keeps_unicode_for_search() {
        let value = serde_json::json!([
            {"type": "input_text", "text": "修复中文搜索"},
            {"type": "output_text", "text": "已完成"}
        ]);
        assert_eq!(first_text(&value), Some("修复中文搜索"));
    }

    #[test]
    fn paginated_history_has_one_native_session_without_main_page() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("codex-pages-{suffix}"));
        std::fs::create_dir_all(&root).unwrap();
        let early = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"id":"page-a","session_id":"thread-1","cwd":"/tmp/a","timestamp":"2026-01-01T00:00:00Z"}}
{"timestamp":"2026-01-01T00:01:00Z","type":"response_item","payload":{"role":"user","content":[{"type":"input_text","text":"Initial task"}]}}"#;
        let late = r#"{"timestamp":"2026-01-02T00:00:00Z","type":"session_meta","payload":{"id":"page-b","session_id":"thread-1","cwd":"/tmp/a","timestamp":"2026-01-02T00:00:00Z"}}
{"timestamp":"2026-01-02T00:01:00Z","type":"response_item","payload":{"role":"user","content":[{"type":"input_text","text":"Follow-up"}]}}"#;
        std::fs::write(root.join("z.jsonl"), early).unwrap();
        std::fs::write(root.join("a.jsonl"), late).unwrap();
        std::fs::write(root.join("duplicate.jsonl"), late).unwrap();
        std::fs::write(
            root.join("bad.jsonl"),
            r#"{"type":"session_meta","payload":{"cwd":"/tmp/a"}}"#,
        )
        .unwrap();
        let adapter = CodexAdapter::new(root.clone(), root.join("missing-index"));
        let (sessions, warnings) = adapter.list_sessions_with_warnings().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "codex:thread-1");
        assert_eq!(sessions[0].title, "Initial task");
        assert!(sessions[0].updated_at_ms > sessions[0].created_at_ms);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("no native session ID"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
