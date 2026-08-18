use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;
use zellij_ai_session_core::{AgentKind, AiSession, CommandSpec};

use crate::adapters::{AgentAdapter, clean_title};

/// Cursor stores one metadata file per chat at
/// `<config>/chats/<workspace>/<chat-id>/meta.json`.
pub struct CursorAdapter {
    chats_root: PathBuf,
}

impl CursorAdapter {
    pub fn new(chats_root: PathBuf) -> Self {
        Self { chats_root }
    }

    fn parse_meta(&self, path: &Path) -> Result<Option<AiSession>> {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("read Cursor session metadata {}", path.display()))?;
        let meta: CursorMeta = serde_json::from_str(&contents)
            .with_context(|| format!("parse Cursor session metadata {}", path.display()))?;

        // Cursor creates metadata directories before a conversation exists.
        if !meta.has_conversation {
            return Ok(None);
        }
        let directory = match meta.cwd {
            Some(directory) if !directory.as_os_str().is_empty() => directory,
            _ => return Ok(None),
        };
        let session_id = path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if session_id.is_empty() {
            return Ok(None);
        }

        let title = clean_title(&meta.title);
        let title = if title.is_empty() {
            format!("Cursor session {}", &session_id[..session_id.len().min(8)])
        } else {
            title
        };

        Ok(Some(AiSession::new(
            AgentKind::Cursor,
            session_id,
            title,
            directory,
            meta.created_at_ms,
            meta.updated_at_ms,
        )))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CursorMeta {
    #[serde(default)]
    has_conversation: bool,
    #[serde(default)]
    title: String,
    #[serde(default)]
    cwd: Option<PathBuf>,
    #[serde(default)]
    created_at_ms: Option<i64>,
    #[serde(default)]
    updated_at_ms: Option<i64>,
}

impl AgentAdapter for CursorAdapter {
    fn name(&self) -> &'static str {
        "Cursor"
    }

    fn agent(&self) -> AgentKind {
        AgentKind::Cursor
    }

    fn list_sessions(&self) -> Result<Vec<AiSession>> {
        let mut sessions = Vec::new();
        if !self.chats_root.exists() {
            return Ok(sessions);
        }
        visit_meta_files(&self.chats_root, &mut |path| {
            if let Some(session) = self.parse_meta(path)? {
                sessions.push(session);
            }
            Ok(())
        })?;
        Ok(sessions)
    }

    fn resume_command(&self, session: &AiSession) -> Result<CommandSpec> {
        Ok(CommandSpec::new("cursor-agent", session.directory.clone())
            .with_args(["--resume", session.agent_session_id.as_str()]))
    }
}

fn visit_meta_files(root: &Path, callback: &mut impl FnMut(&Path) -> Result<()>) -> Result<()> {
    for entry in std::fs::read_dir(root).with_context(|| format!("read {}", root.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            visit_meta_files(&path, callback)?;
        } else if path.file_name().and_then(|name| name.to_str()) == Some("meta.json") {
            callback(&path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "zais-cursor-{}-{}",
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
    fn reads_nonempty_metadata_and_skips_empty_chats() {
        let root = temp_root("metadata");
        let workspace = root.join("workspace");
        let chat = workspace.join("12345678-1234-1234-1234-123456789abc");
        let empty = workspace.join("empty-chat");
        std::fs::create_dir_all(&chat).unwrap();
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::write(
            chat.join("meta.json"),
            r#"{"schemaVersion":1,"createdAtMs":100,"hasConversation":true,"title":"修复 中文 搜索","updatedAtMs":200,"cwd":"/tmp/project"}"#,
        )
        .unwrap();
        std::fs::write(
            empty.join("meta.json"),
            r#"{"schemaVersion":1,"createdAtMs":300,"hasConversation":false,"title":"空会话","updatedAtMs":400,"cwd":"/tmp/project"}"#,
        )
        .unwrap();

        let sessions = CursorAdapter::new(root.clone()).list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].agent, AgentKind::Cursor);
        assert_eq!(sessions[0].title, "修复 中文 搜索");
        assert_eq!(
            sessions[0].agent_session_id,
            "12345678-1234-1234-1234-123456789abc"
        );
        assert_eq!(sessions[0].directory, PathBuf::from("/tmp/project"));
        assert_eq!(sessions[0].created_at_ms, Some(100));
        assert_eq!(sessions[0].updated_at_ms, Some(200));

        let command = CursorAdapter::new(root)
            .resume_command(&sessions[0])
            .unwrap();
        assert_eq!(command.program, "cursor-agent");
        assert_eq!(
            command.args,
            vec!["--resume", "12345678-1234-1234-1234-123456789abc"]
        );
    }
}
