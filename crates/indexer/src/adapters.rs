use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::Result;
use serde_json::Value;
use zellij_ai_session_core::{AgentKind, AiSession, CommandSpec, PreviewMessage, SessionPreview};

pub trait AgentAdapter: Send + Sync {
    fn name(&self) -> &'static str;
    fn agent(&self) -> AgentKind;
    fn list_sessions(&self) -> Result<Vec<AiSession>>;
    fn list_sessions_with_warnings(&self) -> Result<(Vec<AiSession>, Vec<String>)> {
        self.list_sessions().map(|sessions| (sessions, Vec::new()))
    }
    fn resume_command(&self, session: &AiSession) -> Result<CommandSpec>;
    fn rename_session(&self, _session_id: &str, _title: &str) -> Result<()> {
        anyhow::bail!(
            "Workbench has no native rename integration for {}",
            self.name()
        )
    }
    fn preview(&self, session_id: &str) -> Result<SessionPreview> {
        Ok(SessionPreview {
            session_id: session_id.into(),
            messages: Vec::new(),
            note: Some(format!("{} preview is not supported", self.name())),
        })
    }
}

pub(crate) fn preview_result(
    session_id: &str,
    found: bool,
    messages: Vec<PreviewMessage>,
) -> SessionPreview {
    let messages = messages.into_iter().rev().take(8).collect::<Vec<_>>();
    let messages = messages.into_iter().rev().collect::<Vec<_>>();
    let note = if !found {
        Some("Native session was not found".into())
    } else if messages.is_empty() {
        Some("No readable user or Agent messages have been saved yet".into())
    } else {
        None
    };
    SessionPreview {
        session_id: session_id.into(),
        messages,
        note,
    }
}

pub(crate) fn preview_message(role: &str, content: &Value) -> Option<PreviewMessage> {
    if !matches!(role, "user" | "assistant") {
        return None;
    }
    let text = first_text(content)?.trim();
    if text.is_empty() {
        return None;
    }
    Some(PreviewMessage {
        role: role.into(),
        text: text.into(),
    })
}

pub(crate) fn preview_project_jsonl(root: &Path, session_id: &str) -> Result<SessionPreview> {
    if !root.is_dir() {
        return Ok(preview_result(session_id, false, Vec::new()));
    }
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
                let file_id = path.file_stem().and_then(|stem| stem.to_str());
                let native_id = BufReader::new(File::open(&path)?)
                    .lines()
                    .take(50)
                    .filter_map(|line| line.ok())
                    .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
                    .find_map(|value| {
                        value
                            .get("sessionId")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    });
                if native_id.as_deref().or(file_id) != Some(session_id) {
                    continue;
                }
                let mut messages = Vec::new();
                for line in BufReader::new(File::open(&path)?).lines() {
                    let Ok(value) = serde_json::from_str::<Value>(&line?) else {
                        continue;
                    };
                    if value.get("isMeta").and_then(Value::as_bool) == Some(true) {
                        continue;
                    }
                    let role = value
                        .get("message")
                        .and_then(|m| m.get("role"))
                        .and_then(Value::as_str)
                        .or_else(|| value.get("type").and_then(Value::as_str));
                    let content = value
                        .get("message")
                        .and_then(|m| m.get("content").or_else(|| m.get("parts")));
                    if let (Some(role), Some(content)) = (role, content) {
                        if let Some(message) = preview_message(role, content) {
                            messages.push(message);
                        }
                    }
                }
                return Ok(preview_result(session_id, true, messages));
            }
        }
    }
    Ok(preview_result(session_id, false, Vec::new()))
}

/// Parse an RFC 3339 / ISO 8601 timestamp (e.g. `2024-12-03T14:00:00.000Z`)
/// into epoch milliseconds.
pub(crate) fn iso_to_ms(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|datetime| datetime.timestamp_millis())
}

/// Last-modified time of a file as epoch milliseconds.
pub(crate) fn file_mtime_ms(path: &Path) -> Option<i64> {
    std::fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
}

/// Extract the first plain-text string from a message `content` value, which
/// may be a string, an array of content blocks, or an object with a `text`
/// field.
pub(crate) fn first_text(value: &Value) -> Option<&str> {
    match value {
        Value::String(text) => Some(text),
        Value::Array(values) => values.iter().find_map(first_text),
        Value::Object(object) => object.get("text").and_then(Value::as_str),
        _ => None,
    }
}

/// Collapse whitespace and truncate to 200 chars.
pub(crate) fn collapse(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(200)
        .collect()
}

/// Collapse whitespace and truncate to 200 chars; returns `None` if empty.
pub(crate) fn collapse_nonempty(text: &str) -> Option<String> {
    let collapsed = collapse(text);
    if collapsed.is_empty() {
        None
    } else {
        Some(collapsed)
    }
}

fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.find(open)? + open.len();
    let end = text[start..].find(close)? + start;
    Some(&text[start..end])
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Best-effort title for an agent user prompt. Strips ANSI escapes and extracts
/// the meaningful text from command-invocation wrappers (`<command-message>`,
/// `<local-command-stdout>`, ...). Returns `None` for ordinary prompts so the
/// caller falls back to plain whitespace cleanup.
pub(crate) fn structured_title(text: &str) -> Option<String> {
    let text = strip_ansi(text);
    if let Some(inner) = between(&text, "<command-message>", "</command-message>") {
        let mut title = inner.trim().to_string();
        if let Some(args) = between(&text, "<command-args>", "</command-args>") {
            let args = args.trim();
            if !args.is_empty() {
                title.push(' ');
                title.push_str(args);
            }
        }
        return collapse_nonempty(&title);
    }
    if let Some(inner) = between(&text, "<command-name>", "</command-name>") {
        return collapse_nonempty(inner.trim());
    }
    if let Some(inner) = between(&text, "<local-command-stdout>", "</local-command-stdout>") {
        return collapse_nonempty(inner.trim());
    }
    None
}

/// Whitespace-collapse a title string (used as the plain fallback).
pub(crate) fn clean_title(text: &str) -> String {
    collapse(text)
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AdapterContext;
