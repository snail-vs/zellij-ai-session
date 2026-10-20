use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;

use zellij_ai_session_core::{AgentKind, AiSession, CommandSpec, SessionPreview};

use crate::adapters::{AgentAdapter, preview_result};
use crate::pi_format;

pub struct PiAdapter {
    sessions_root: PathBuf,
}

impl PiAdapter {
    pub fn new(sessions_root: PathBuf) -> Self {
        Self { sessions_root }
    }

    fn parse_file(&self, path: &Path) -> Result<Option<AiSession>> {
        let Some(header) = pi_format::read_header(path)? else {
            return Ok(None);
        };
        if header.cwd.as_os_str().is_empty() {
            return Ok(None);
        }
        let title = header
            .title
            .unwrap_or_else(|| format!("Pi session {}", &header.id[..header.id.len().min(8)]));
        // Pi addresses its sessions by file path, not by the uuid in the header.
        let agent_session_id = path.to_string_lossy().into_owned();
        Ok(Some(AiSession::new(
            AgentKind::Pi,
            &agent_session_id,
            title,
            header.cwd,
            header.created_at_ms,
            header.updated_at_ms,
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

    fn rename_session(&self, session: &AiSession, title: &str) -> Result<()> {
        let path = Path::new(&session.agent_session_id);
        anyhow::ensure!(
            path.extension().and_then(|ext| ext.to_str()) == Some("jsonl")
                && path.canonicalize().ok().is_some_and(|path| {
                    self.sessions_root
                        .canonicalize()
                        .ok()
                        .is_some_and(|root| path.starts_with(root))
                }),
            "Pi session file is outside the configured session directory"
        );
        let output = Command::new("pi")
            .args([
                "--session",
                &session.agent_session_id,
                "--name",
                title,
                "--print",
                "--offline",
                "--no-extensions",
                "--no-context-files",
            ])
            .current_dir(&session.directory)
            .output()?;
        anyhow::ensure!(
            output.status.success(),
            "Pi rename failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        Ok(())
    }

    fn list_sessions(&self) -> Result<Vec<AiSession>> {
        let mut result = Vec::new();
        if !self.sessions_root.exists() {
            return Ok(result);
        }
        for jsonl in pi_format::session_files(&self.sessions_root) {
            if let Some(session) = self.parse_file(&jsonl)? {
                result.push(session);
            }
        }
        Ok(result)
    }

    fn resume_command(&self, session: &AiSession) -> Result<CommandSpec> {
        Ok(CommandSpec::new("pi", session.directory.clone())
            .with_args(["--session", session.agent_session_id.as_str()]))
    }

    fn preview(&self, session_id: &str) -> Result<SessionPreview> {
        let path = Path::new(session_id);
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl")
            || !path.canonicalize().ok().is_some_and(|path| {
                self.sessions_root
                    .canonicalize()
                    .ok()
                    .is_some_and(|root| path.starts_with(root))
            })
        {
            return Ok(preview_result(session_id, false, Vec::new()));
        }
        let messages = pi_format::read_messages(path)?;
        Ok(preview_result(session_id, true, messages))
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
