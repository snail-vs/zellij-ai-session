use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;
use zellij_ai_session_core::{
    AgentKind, AiSession, CommandSpec, SessionStatus, project_for_directory,
};

use crate::adapters::AgentAdapter;

const SEARCH_TEXT_LIMIT: usize = 64 * 1024;

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

    fn parse_file(&self, path: &Path) -> Result<Option<AiSession>> {
        let file =
            File::open(path).with_context(|| format!("open Codex session {}", path.display()))?;
        let reader = BufReader::new(file);
        let mut meta = None;
        let mut title = None;
        let mut search_text = String::new();
        let mut last_timestamp = None;

        for line in reader.lines() {
            let line = line?;
            let value: Value = match serde_json::from_str(&line) {
                Ok(value) => value,
                Err(_) => continue,
            };
            last_timestamp = value
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(parse_timestamp);
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
                        if let Some(content) = content {
                            append_json_text(&mut search_text, content);
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
        let session_id = payload
            .get("session_id")
            .or_else(|| payload.get("id"))
            .and_then(Value::as_str)
            .or_else(|| path.file_stem().and_then(|name| name.to_str()))
            .unwrap_or_default();
        let directory = payload
            .get("cwd")
            .and_then(Value::as_str)
            .map(PathBuf::from);
        let directory = match directory {
            Some(directory) => directory,
            None => return Ok(None),
        };
        let project = project_for_directory(&directory);
        let created_at_ms = payload
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_timestamp)
            .or_else(|| value_timestamp(&meta));
        let updated_at_ms = last_timestamp.or(created_at_ms);

        Ok(Some(AiSession {
            id: format!("codex:{session_id}"),
            agent: AgentKind::Codex,
            title: title.unwrap_or_else(|| {
                format!("Codex session {}", &session_id[..session_id.len().min(8)])
            }),
            search_text,
            project_id: project.id,
            directory,
            created_at_ms,
            updated_at_ms,
            agent_session_id: session_id.to_string(),
            status: SessionStatus::Historical,
            runtime: None,
        }))
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
        let mut result = Vec::new();
        if !self.sessions_root.exists() {
            return Ok(result);
        }
        let thread_names = read_thread_names(&self.session_index)?;
        visit_jsonl(&self.sessions_root, &mut |path| {
            if let Some(mut session) = self.parse_file(path)? {
                if let Some(thread_name) = thread_names.get(&session.agent_session_id) {
                    session.title = thread_name.clone();
                }
                result.push(session);
            }
            Ok(())
        })?;
        Ok(result)
    }

    fn resume_command(&self, session: &AiSession) -> Result<CommandSpec> {
        Ok(CommandSpec::new("codex", session.directory.clone())
            .with_args(["resume", session.agent_session_id.as_str()]))
    }
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
    let (date, time) = value.split_once('T')?;
    let (year, rest) = date.split_once('-')?;
    let (month, day) = rest.split_once('-')?;
    let (hour, rest) = time.split_once(':')?;
    let (minute, rest) = rest.split_once(':')?;
    let second = rest
        .split(|ch| ch == '.' || ch == 'Z' || ch == '+' || ch == '-')
        .next()?;
    let days = days_from_civil(year.parse().ok()?, month.parse().ok()?, day.parse().ok()?);
    Some(
        (((days * 24 + hour.parse::<i64>().ok()?) * 60 + minute.parse::<i64>().ok()?) * 60
            + second.parse::<i64>().ok()?)
            * 1000,
    )
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = (if year >= 0 { year } else { year - 399 }) / 400;
    let year_of_era = year - era * 400;
    let month_adjusted = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_adjusted + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146097 + day_of_era - 719468
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

fn append_json_text(output: &mut String, value: &Value) {
    if output.len() >= SEARCH_TEXT_LIMIT {
        return;
    }
    match value {
        Value::String(text) => append_text(output, text),
        Value::Array(values) => {
            for value in values {
                append_json_text(output, value);
                if output.len() >= SEARCH_TEXT_LIMIT {
                    break;
                }
            }
        }
        Value::Object(object) => {
            if let Some(text) = object.get("text").and_then(Value::as_str) {
                append_text(output, text);
            }
        }
        _ => {}
    }
}

fn append_text(output: &mut String, text: &str) {
    let text = text.trim();
    if text.is_empty() || output.len() >= SEARCH_TEXT_LIMIT {
        return;
    }
    if !output.is_empty() {
        output.push('\n');
    }
    let remaining = SEARCH_TEXT_LIMIT.saturating_sub(output.len());
    let end = text
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(text.len()))
        .take_while(|index| *index <= remaining)
        .last()
        .unwrap_or_default();
    output.push_str(&text[..end]);
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn codex_content_keeps_unicode_for_search() {
        let value = serde_json::json!([
            {"type": "input_text", "text": "修复中文搜索"},
            {"type": "output_text", "text": "已完成"}
        ]);
        let mut search_text = String::new();
        append_json_text(&mut search_text, &value);
        assert!(search_text.contains("中文搜索"));
        assert!(search_text.contains("已完成"));
    }
}
