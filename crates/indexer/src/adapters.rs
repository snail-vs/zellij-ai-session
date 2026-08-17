use std::path::Path;

use anyhow::Result;
use serde_json::Value;
use zellij_ai_session_core::{AgentKind, AiSession, CommandSpec};

pub trait AgentAdapter: Send + Sync {
    fn name(&self) -> &'static str;
    fn agent(&self) -> AgentKind;
    fn list_sessions(&self) -> Result<Vec<AiSession>>;
    fn resume_command(&self, session: &AiSession) -> Result<CommandSpec>;
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
