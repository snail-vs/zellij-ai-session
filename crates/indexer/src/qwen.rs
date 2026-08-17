use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

use zellij_ai_session_core::{AgentKind, AiSession, CommandSpec};

use crate::adapters::{AgentAdapter, clean_title, first_text, iso_to_ms, structured_title};

pub struct QwenAdapter {
    projects_dir: PathBuf,
}

impl QwenAdapter {
    pub fn new(projects_dir: PathBuf) -> Self {
        Self { projects_dir }
    }

    fn parse_file(&self, path: &Path) -> Result<Option<AiSession>> {
        let file =
            File::open(path).with_context(|| format!("open Qwen session {}", path.display()))?;
        let reader = BufReader::new(file);

        let mut cwd: Option<String> = None;
        let mut session_id: Option<String> = None;
        let mut created_at_ms: Option<i64> = None;
        let mut updated_at_ms: Option<i64> = None;
        let mut title: Option<String> = None;
        let mut command_fallback: Option<String> = None;

        for line in reader.lines() {
            let line = line.with_context(|| format!("read Qwen session {}", path.display()))?;
            let entry: Value = match serde_json::from_str(&line) {
                Ok(value) => value,
                Err(_) => continue,
            };

            if let Some(value) = entry.get("cwd").and_then(Value::as_str) {
                if !value.is_empty() && cwd.is_none() {
                    cwd = Some(value.to_string());
                }
            }
            if let Some(value) = entry.get("sessionId").and_then(Value::as_str) {
                if session_id.is_none() {
                    session_id = Some(value.to_string());
                }
            }
            if let Some(value) = entry
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(iso_to_ms)
            {
                if created_at_ms.is_none() {
                    created_at_ms = Some(value);
                }
                updated_at_ms = Some(value);
            }

            if title.is_none() {
                let is_user = entry.get("type").and_then(Value::as_str) == Some("user");
                let is_meta = entry
                    .get("isMeta")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if is_user {
                    if let Some(text) = user_text(&entry) {
                        if let Some(command) = structured_title(&text) {
                            if command_fallback.is_none() {
                                command_fallback = Some(command);
                            }
                        } else {
                            let cleaned = clean_title(&text);
                            if !cleaned.is_empty() && !is_meta {
                                title = Some(cleaned);
                            }
                        }
                    }
                }
            }
        }

        let id = session_id.unwrap_or_else(|| {
            path.file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string()
        });
        let directory: PathBuf = match cwd {
            Some(directory) if !directory.is_empty() => directory.into(),
            _ => return Ok(None),
        };
        let title = title
            .or(command_fallback)
            .unwrap_or_else(|| format!("Qwen session {}", &id[..id.len().min(8)]));

        Ok(Some(AiSession::new(
            AgentKind::Qwen,
            &id,
            title,
            directory,
            created_at_ms,
            updated_at_ms,
        )))
    }
}

/// Plain-text of the first user prompt. Qwen stores user content in
/// `message.parts` (an array of `{text}`), unlike Claude's `message.content`,
/// so we accept either.
fn user_text(entry: &Value) -> Option<String> {
    let message = entry.get("message")?;
    message
        .get("content")
        .or_else(|| message.get("parts"))
        .and_then(first_text)
        .map(|text| text.to_string())
}

impl AgentAdapter for QwenAdapter {
    fn name(&self) -> &'static str {
        "Qwen"
    }

    fn agent(&self) -> AgentKind {
        AgentKind::Qwen
    }

    fn list_sessions(&self) -> Result<Vec<AiSession>> {
        let mut result = Vec::new();
        if !self.projects_dir.exists() {
            return Ok(result);
        }
        for entry in std::fs::read_dir(&self.projects_dir)
            .with_context(|| format!("read {}", self.projects_dir.display()))?
        {
            let entry = entry?;
            let project_dir = entry.path();
            if !project_dir.is_dir() {
                continue;
            }
            let mut scan = |dir: &Path| -> Result<()> {
                for file in std::fs::read_dir(dir)? {
                    let path = file?.path();
                    if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
                        if let Some(session) = self.parse_file(&path)? {
                            result.push(session);
                        }
                    }
                }
                Ok(())
            };
            scan(&project_dir)?;
            if project_dir.join("chats").is_dir() {
                scan(&project_dir.join("chats"))?;
            }
            if project_dir.join("sessions").is_dir() {
                scan(&project_dir.join("sessions"))?;
            }
        }
        Ok(result)
    }

    fn resume_command(&self, session: &AiSession) -> Result<CommandSpec> {
        Ok(CommandSpec::new("qwen", session.directory.clone())
            .with_args(["--resume", session.agent_session_id.as_str()]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "zais-qwen-{}-{}",
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
    fn reads_parts_content_cwd_and_timestamps() {
        let root = temp_root("parts");
        let project = root.join("-data-code-elh-assistant");
        std::fs::create_dir_all(project.join("chats")).unwrap();
        let session = project.join("chats/ec6b1fb4-a003-4f55-8552-b83891474e4e.jsonl");
        std::fs::write(
            &session,
            r#"{"type":"system","sessionId":"ec6b1fb4-a003-4f55-8552-b83891474e4e","cwd":"/data/code/elh-assistant","timestamp":"2026-07-04T09:02:42.350Z","version":"0.19.6"}
{"type":"user","sessionId":"ec6b1fb4-a003-4f55-8552-b83891474e4e","cwd":"/data/code/elh-assistant","timestamp":"2026-07-04T09:03:00.000Z","message":{"role":"user","parts":[{"text":"可用吗？可用回复ok"}]}}
{"type":"assistant","sessionId":"ec6b1fb4-a003-4f55-8552-b83891474e4e","cwd":"/data/code/elh-assistant","timestamp":"2026-07-04T09:03:30.000Z","message":{"role":"assistant","parts":[{"text":"ok"}]}}"#,
        )
        .unwrap();

        let sessions = QwenAdapter::new(root.clone()).list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.agent, AgentKind::Qwen);
        assert_eq!(session.title, "可用吗？可用回复ok");
        assert_eq!(session.directory, PathBuf::from("/data/code/elh-assistant"));
        assert_eq!(session.created_at_ms, Some(1_783_155_762_350));
        assert_eq!(session.updated_at_ms, Some(1_783_155_810_000));
        assert_eq!(
            session.agent_session_id,
            "ec6b1fb4-a003-4f55-8552-b83891474e4e"
        );

        let command = QwenAdapter::new(root.clone())
            .resume_command(session)
            .unwrap();
        assert_eq!(command.program, "qwen");
        assert_eq!(
            command.args,
            vec!["--resume", session.agent_session_id.as_str()]
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn skips_sessions_without_cwd() {
        let root = temp_root("nocwd");
        let project = root.join("-data-proj");
        std::fs::create_dir_all(&project).unwrap();
        let session = project.join("aaaaaaaa-1111-2222-3333-444444444444.jsonl");
        std::fs::write(
            &session,
            r#"{"type":"user","sessionId":"aaaaaaaa-1111-2222-3333-444444444444","timestamp":"2026-07-04T09:03:00.000Z","message":{"role":"user","parts":[{"text":"没有 cwd"}]}}"#,
        )
        .unwrap();

        let sessions = QwenAdapter::new(root.clone()).list_sessions().unwrap();
        assert!(sessions.is_empty());

        std::fs::remove_dir_all(&root).ok();
    }
}
