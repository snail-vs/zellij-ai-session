use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

use zellij_ai_session_core::{AgentKind, AiSession, CommandSpec};

use crate::adapters::AgentAdapter;

pub struct PiAdapter {
    sessions_root: PathBuf,
}

impl PiAdapter {
    pub fn new(sessions_root: PathBuf) -> Self {
        Self { sessions_root }
    }

    fn parse_file(&self, path: &Path) -> Result<Option<AiSession>> {
        let file =
            File::open(path).with_context(|| format!("open Pi session {}", path.display()))?;
        let mut lines = BufReader::new(file).lines();

        let header_line = match lines.next().transpose()? {
            Some(line) => line,
            None => return Ok(None),
        };
        let header: Value = serde_json::from_str(&header_line)
            .with_context(|| format!("parse Pi session header {}", path.display()))?;
        if header.get("type").and_then(Value::as_str) != Some("session") {
            return Ok(None);
        }
        let uuid = header
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let directory: PathBuf = match header.get("cwd").and_then(Value::as_str) {
            Some(cwd) => cwd.into(),
            None => return Ok(None),
        };
        let created_at_ms = header
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(crate::adapters::iso_to_ms);

        let mut title: Option<String> = None;
        let mut updated_at_ms = created_at_ms;
        for line in lines {
            let line = line?;
            let entry: Value = match serde_json::from_str(&line) {
                Ok(value) => value,
                Err(_) => continue,
            };
            match entry.get("type").and_then(Value::as_str) {
                Some("session_info") => {
                    if let Some(name) = entry.get("name").and_then(Value::as_str) {
                        let cleaned = name.lines().next().unwrap_or(name).trim().to_string();
                        if !cleaned.is_empty() {
                            title = Some(cleaned.chars().take(200).collect());
                        }
                    }
                }
                Some("message") => {
                    if title.is_none() {
                        if let Some(message) = entry.get("message") {
                            if message.get("role").and_then(Value::as_str) == Some("user") {
                                title = message
                                    .get("content")
                                    .and_then(crate::adapters::first_text)
                                    .map(|text| text.chars().take(200).collect());
                            }
                        }
                    }
                    if let Some(timestamp) = entry
                        .get("timestamp")
                        .and_then(Value::as_str)
                        .and_then(crate::adapters::iso_to_ms)
                    {
                        updated_at_ms = Some(timestamp);
                    }
                }
                _ => {}
            }
        }

        let title = title.unwrap_or_else(|| format!("Pi session {}", &uuid[..uuid.len().min(8)]));
        let agent_session_id = path.to_string_lossy().into_owned();
        Ok(Some(AiSession::new(
            AgentKind::Pi,
            &agent_session_id,
            title,
            directory,
            created_at_ms,
            updated_at_ms,
        )))
    }
}

impl AgentAdapter for PiAdapter {
    fn name(&self) -> &'static str {
        "Pi"
    }

    fn agent(&self) -> AgentKind {
        AgentKind::Pi
    }

    fn list_sessions(&self) -> Result<Vec<AiSession>> {
        let mut result = Vec::new();
        if !self.sessions_root.exists() {
            return Ok(result);
        }
        for entry in std::fs::read_dir(&self.sessions_root)
            .with_context(|| format!("read {}", self.sessions_root.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            let jsonl_files: Vec<PathBuf> = if path.is_dir() {
                std::fs::read_dir(&path)
                    .with_context(|| format!("read {}", path.display()))?
                    .filter_map(|sub| sub.ok().map(|sub| sub.path()))
                    .filter(|sub| sub.extension().and_then(|ext| ext.to_str()) == Some("jsonl"))
                    .collect()
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
                vec![path]
            } else {
                continue;
            };
            for jsonl in jsonl_files {
                if let Some(session) = self.parse_file(&jsonl)? {
                    result.push(session);
                }
            }
        }
        Ok(result)
    }

    fn resume_command(&self, session: &AiSession) -> Result<CommandSpec> {
        Ok(CommandSpec::new("pi", session.directory.clone())
            .with_args(["--session", session.agent_session_id.as_str()]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "zais-pi-{}-{}",
            name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn lists_session_with_title_and_cwd() {
        let root = temp_root("list");
        let project_dir = root.join("--home-user-project--");
        std::fs::create_dir_all(&project_dir).unwrap();
        let session = project_dir.join("20260101-000000-abc.jsonl");
        std::fs::write(
            &session,
            r#"{"type":"session","version":3,"id":"abc-123","timestamp":"2026-01-01T10:00:00.000Z","cwd":"/home/user/project"}
{"type":"session_info","name":"Refactor auth"}
{"type":"message","id":"m1","parentId":null,"timestamp":"2026-01-01T10:00:01.000Z","message":{"role":"user","content":"first message"}}
{"type":"message","id":"m2","parentId":"m1","timestamp":"2026-01-01T10:05:00.000Z","message":{"role":"assistant","content":[{"type":"text","text":"done"}]}}
"#,
        )
        .unwrap();

        let adapter = PiAdapter::new(root.clone());
        let sessions = adapter.list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.agent, AgentKind::Pi);
        assert_eq!(session.title, "Refactor auth");
        assert_eq!(session.directory, PathBuf::from("/home/user/project"));
        assert_eq!(session.created_at_ms, Some(1_767_261_600_000));
        assert_eq!(session.updated_at_ms, Some(1_767_261_900_000));
        assert!(session.agent_session_id.ends_with("abc.jsonl"));

        let command = adapter.resume_command(session).unwrap();
        assert_eq!(command.program, "pi");
        assert_eq!(
            command.args,
            vec!["--session", session.agent_session_id.as_str()]
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn falls_back_to_first_user_message_for_title() {
        let root = temp_root("title");
        let project_dir = root.join("--home-user-project--");
        std::fs::create_dir_all(&project_dir).unwrap();
        let session = project_dir.join("20260101-000000-def.jsonl");
        std::fs::write(
            &session,
            r#"{"type":"session","version":3,"id":"def-456","timestamp":"2026-01-01T10:00:00.000Z","cwd":"/home/user/project"}
{"type":"message","id":"m1","parentId":null,"timestamp":"2026-01-01T10:00:01.000Z","message":{"role":"user","content":"中文搜索功能"}}
"#,
        )
        .unwrap();

        let sessions = PiAdapter::new(root.clone()).list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "中文搜索功能");

        std::fs::remove_dir_all(&root).ok();
    }
}
