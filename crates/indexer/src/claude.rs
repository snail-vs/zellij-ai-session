use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use serde_json::Value;

use zellij_ai_session_core::{AgentKind, AiSession, CommandSpec, SessionPreview};

use crate::adapters::{AgentAdapter, clean_title, preview_project_jsonl, structured_title};

pub struct ClaudeAdapter {
    projects_dir: PathBuf,
}

impl ClaudeAdapter {
    pub fn new(projects_dir: PathBuf) -> Self {
        Self { projects_dir }
    }

    fn parse_file(&self, path: &Path) -> Result<Option<AiSession>> {
        let file =
            File::open(path).with_context(|| format!("open Claude session {}", path.display()))?;
        let reader = BufReader::new(file);

        let mut cwd: Option<String> = None;
        let mut session_id: Option<String> = None;
        let mut created_at_ms: Option<i64> = None;
        let mut updated_at_ms: Option<i64> = None;
        let mut title: Option<String> = None;
        let mut custom_title: Option<String> = None;
        let mut command_fallback: Option<String> = None;

        for line in reader.lines() {
            let line = line.with_context(|| format!("read Claude session {}", path.display()))?;
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
                .and_then(crate::adapters::iso_to_ms)
            {
                if created_at_ms.is_none() {
                    created_at_ms = Some(value);
                }
                updated_at_ms = Some(value);
            }

            if entry.get("type").and_then(Value::as_str) == Some("custom-title") {
                let record_id = entry.get("sessionId").and_then(Value::as_str);
                let file_id = path.file_stem().and_then(|stem| stem.to_str());
                if record_id.is_none() || record_id == session_id.as_deref().or(file_id) {
                    custom_title = entry
                        .get("customTitle")
                        .and_then(Value::as_str)
                        .map(clean_title)
                        .filter(|title| !title.is_empty());
                }
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
        let title = custom_title
            .or(title)
            .or(command_fallback)
            .unwrap_or_else(|| format!("Claude session {}", &id[..id.len().min(8)]));

        Ok(Some(AiSession::new(
            AgentKind::Claude,
            &id,
            title,
            directory,
            created_at_ms,
            updated_at_ms,
        )))
    }
}

/// Plain-text of the first user prompt, used as the session title.
fn user_text(entry: &Value) -> Option<String> {
    entry
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(crate::adapters::first_text)
        .map(|text| text.to_string())
        .or_else(|| {
            entry
                .get("message")
                .and_then(crate::adapters::first_text)
                .map(|text| text.to_string())
        })
}

impl AgentAdapter for ClaudeAdapter {
    fn name(&self) -> &'static str {
        "Claude"
    }

    fn agent(&self) -> AgentKind {
        AgentKind::Claude
    }

    fn rename_session(&self, session: &AiSession, title: &str) -> Result<()> {
        let output = Command::new("claude")
            .args([
                "--print",
                "--resume",
                &session.agent_session_id,
                &format!("/rename {title}"),
            ])
            .current_dir(&session.directory)
            .output()?;
        anyhow::ensure!(
            output.status.success(),
            "Claude rename failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        Ok(())
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
            if project_dir.join("sessions").is_dir() {
                scan(&project_dir.join("sessions"))?;
            }
        }
        Ok(result)
    }

    fn resume_command(&self, session: &AiSession) -> Result<CommandSpec> {
        Ok(CommandSpec::new("claude", session.directory.clone())
            .with_args(["--resume", session.agent_session_id.as_str()]))
    }

    fn preview(&self, session_id: &str) -> Result<SessionPreview> {
        preview_project_jsonl(&self.projects_dir, session_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "zais-claude-{}-{}",
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
    fn reads_cwd_session_id_and_timestamps() {
        let root = temp_root("cwd");
        let project = root.join("-home-snail");
        std::fs::create_dir_all(&project).unwrap();
        let session = project.join("110b8e3b-1e41-4a11-9df3-366d7704cc33.jsonl");
        std::fs::write(
            &session,
            r#"{"type":"system","sessionId":"110b8e3b-1e41-4a11-9df3-366d7704cc33","cwd":"/home/snail","timestamp":"2026-07-29T13:47:46.920Z","version":"2.1.201"}
{"type":"user","isMeta":true,"sessionId":"110b8e3b-1e41-4a11-9df3-366d7704cc33","cwd":"/home/snail","timestamp":"2026-07-29T13:47:47.000Z","message":{"role":"user","content":"<command-message>claude-api</command-message>\n<command-name>/claude-api</command-name>"}}
{"type":"user","sessionId":"110b8e3b-1e41-4a11-9df3-366d7704cc33","cwd":"/home/snail","timestamp":"2026-07-29T13:48:10.000Z","message":{"role":"user","content":"帮我把中文搜索修好"}}
{"type":"assistant","sessionId":"110b8e3b-1e41-4a11-9df3-366d7704cc33","cwd":"/home/snail","timestamp":"2026-07-29T13:49:00.000Z","message":{"role":"assistant","content":[{"type":"text","text":"好的"}]}}"#,
        )
        .unwrap();

        let sessions = ClaudeAdapter::new(root.clone()).list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.agent, AgentKind::Claude);
        assert_eq!(session.title, "帮我把中文搜索修好");
        assert_eq!(session.directory, PathBuf::from("/home/snail"));
        assert_eq!(session.created_at_ms, Some(1_785_332_866_920));
        assert_eq!(session.updated_at_ms, Some(1_785_332_940_000));
        assert_eq!(
            session.agent_session_id,
            "110b8e3b-1e41-4a11-9df3-366d7704cc33"
        );

        let command = ClaudeAdapter::new(root.clone())
            .resume_command(session)
            .unwrap();
        assert_eq!(command.program, "claude");
        assert_eq!(
            command.args,
            vec!["--resume", session.agent_session_id.as_str()]
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn falls_back_to_command_title_when_only_command_prompt() {
        let root = temp_root("cmd");
        let project = root.join("-home-snail");
        std::fs::create_dir_all(&project).unwrap();
        let session = project.join("aaaaaaaa-1111-2222-3333-444444444444.jsonl");
        std::fs::write(
            &session,
            r#"{"type":"system","sessionId":"aaaaaaaa-1111-2222-3333-444444444444","cwd":"/data/proj","timestamp":"2026-07-29T13:47:46.920Z"}
{"type":"user","isMeta":true,"sessionId":"aaaaaaaa-1111-2222-3333-444444444444","cwd":"/data/proj","timestamp":"2026-07-29T13:48:10.000Z","message":{"role":"user","content":"<command-message>claude-api</command-message>\n<command-name>/claude-api</command-name>"}}"#,
        )
        .unwrap();

        let sessions = ClaudeAdapter::new(root.clone()).list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "claude-api");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn native_custom_title_overrides_first_prompt() {
        let root = temp_root("rename");
        let project = root.join("-tmp");
        std::fs::create_dir_all(&project).unwrap();
        let session = project.join("aaaaaaaa-1111-2222-3333-444444444444.jsonl");
        std::fs::write(&session, concat!(
            "{\"type\":\"user\",\"sessionId\":\"aaaaaaaa-1111-2222-3333-444444444444\",\"cwd\":\"/tmp\",\"message\":{\"role\":\"user\",\"content\":\"Before\"}}\n",
            "{\"type\":\"custom-title\",\"sessionId\":\"aaaaaaaa-1111-2222-3333-444444444444\",\"customTitle\":\"After\"}\n"
        )).unwrap();
        let sessions = ClaudeAdapter::new(root.clone()).list_sessions().unwrap();
        assert_eq!(sessions[0].title, "After");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn skips_sessions_without_cwd() {
        let root = temp_root("nocwd");
        let project = root.join("-home-snail");
        std::fs::create_dir_all(&project).unwrap();
        let session = project.join("bbbbbbbb-1111-2222-3333-444444444444.jsonl");
        std::fs::write(
            &session,
            r#"{"type":"user","sessionId":"bbbbbbbb-1111-2222-3333-444444444444","timestamp":"2026-07-29T13:48:10.000Z","message":{"role":"user","content":"没有 cwd"}}"#,
        )
        .unwrap();

        let sessions = ClaudeAdapter::new(root.clone()).list_sessions().unwrap();
        assert!(sessions.is_empty());

        std::fs::remove_dir_all(&root).ok();
    }
}
