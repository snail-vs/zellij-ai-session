use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;
use zellij_ai_session_core::{
    AgentKind, AiSession, CommandSpec, SessionStatus, project_for_directory,
};

use crate::adapters::AgentAdapter;

pub struct CodexAdapter {
    sessions_root: PathBuf,
}

impl CodexAdapter {
    pub fn new(sessions_root: PathBuf) -> Self {
        Self { sessions_root }
    }

    fn parse_file(&self, path: &Path) -> Result<Option<AiSession>> {
        let file =
            File::open(path).with_context(|| format!("open Codex session {}", path.display()))?;
        let reader = BufReader::new(file);
        let mut meta = None;
        let mut title = None;
        let mut last_timestamp = None;

        for line in reader.lines().take(256) {
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
                Some("response_item") if title.is_none() => {
                    if value.pointer("/payload/role").and_then(Value::as_str) == Some("user") {
                        title = value
                            .pointer("/payload/content/0/text")
                            .and_then(Value::as_str)
                            .map(clean_title);
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
        visit_jsonl(&self.sessions_root, &mut |path| {
            if let Some(session) = self.parse_file(path)? {
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

fn clean_title(title: &str) -> String {
    let title = title.lines().next().unwrap_or(title).trim();
    if title.is_empty() {
        "Untitled session".into()
    } else {
        title.chars().take(100).collect()
    }
}
