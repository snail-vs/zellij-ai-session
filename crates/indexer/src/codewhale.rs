use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

use zellij_ai_session_core::{AgentKind, AiSession, CommandSpec};

use crate::adapters::AgentAdapter;

pub struct CodewhaleAdapter {
    sessions_dir: PathBuf,
}

impl CodewhaleAdapter {
    pub fn new(sessions_dir: PathBuf) -> Self {
        Self { sessions_dir }
    }

    fn parse_file(&self, path: &Path) -> Result<Option<AiSession>> {
        let file = File::open(path)
            .with_context(|| format!("open Codewhale session {}", path.display()))?;
        let value: Value = serde_json::from_reader(BufReader::new(file))
            .with_context(|| format!("parse Codewhale session {}", path.display()))?;

        let metadata = match value.get("metadata").and_then(Value::as_object) {
            Some(metadata) => metadata,
            None => return Ok(None),
        };
        let directory: PathBuf = match metadata.get("workspace").and_then(Value::as_str) {
            Some(workspace) if !workspace.is_empty() => workspace.into(),
            _ => return Ok(None),
        };

        let id = metadata
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .to_string()
            });
        let title = metadata
            .get("title")
            .and_then(Value::as_str)
            .filter(|title| !title.trim().is_empty())
            .map(|title| title.chars().take(200).collect())
            .or_else(|| first_user_text(&value))
            .unwrap_or_else(|| format!("Codewhale session {}", &id[..id.len().min(8)]));

        let created_at_ms = metadata
            .get("created_at")
            .and_then(Value::as_str)
            .and_then(crate::adapters::iso_to_ms);
        let updated_at_ms = metadata
            .get("updated_at")
            .and_then(Value::as_str)
            .and_then(crate::adapters::iso_to_ms)
            .or(created_at_ms);

        Ok(Some(AiSession::new(
            AgentKind::Codewhale,
            &id,
            title,
            directory,
            created_at_ms,
            updated_at_ms,
        )))
    }
}

/// First plain-text content of the first user message, used as a fallback title.
fn first_user_text(value: &Value) -> Option<String> {
    let messages = value.get("messages").and_then(Value::as_array)?;
    for message in messages {
        if message.get("role").and_then(Value::as_str) == Some("user") {
            if let Some(text) = message.get("content").and_then(crate::adapters::first_text) {
                return Some(text.chars().take(200).collect());
            }
        }
    }
    None
}

impl AgentAdapter for CodewhaleAdapter {
    fn name(&self) -> &'static str {
        "Codewhale"
    }

    fn agent(&self) -> AgentKind {
        AgentKind::Codewhale
    }

    fn list_sessions(&self) -> Result<Vec<AiSession>> {
        let mut result = Vec::new();
        if !self.sessions_dir.exists() {
            return Ok(result);
        }
        for entry in std::fs::read_dir(&self.sessions_dir)
            .with_context(|| format!("read {}", self.sessions_dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            if let Some(session) = self.parse_file(&path)? {
                result.push(session);
            }
        }
        Ok(result)
    }

    fn resume_command(&self, session: &AiSession) -> Result<CommandSpec> {
        Ok(CommandSpec::new("codewhale", session.directory.clone())
            .with_args(["--resume", session.agent_session_id.as_str()]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "zais-cw-{}-{}",
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
    fn reads_metadata_for_title_cwd_and_timestamps() {
        let root = temp_root("meta");
        let session = root.join("c303af53-6400-4c92-a9e1-879d61e08f14.json");
        std::fs::write(
            &session,
            r#"{
  "schema_version": 1,
  "metadata": {
    "id": "c303af53-6400-4c92-a9e1-879d61e08f14",
    "title": "销毁 gitea 然后重建",
    "created_at": "2026-08-03T11:06:26.262794160Z",
    "updated_at": "2026-08-03T11:07:43.654241940Z",
    "workspace": "/data/um880pro",
    "model": "deepseek-v4-flash-free"
  },
  "messages": [
    {"role": "user", "content": [{"type": "text", "text": "销毁 gitea 然后重建"}]},
    {"role": "assistant", "content": [{"type": "text", "text": "好的"}]}
  ]
}"#,
        )
        .unwrap();

        let adapter = CodewhaleAdapter::new(root.clone());
        let sessions = adapter.list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.agent, AgentKind::Codewhale);
        assert_eq!(session.title, "销毁 gitea 然后重建");
        assert_eq!(session.directory, PathBuf::from("/data/um880pro"));
        assert_eq!(session.created_at_ms, Some(1_785_755_186_262));
        assert_eq!(session.updated_at_ms, Some(1_785_755_263_654));
        assert_eq!(
            session.agent_session_id,
            "c303af53-6400-4c92-a9e1-879d61e08f14"
        );

        let command = adapter.resume_command(session).unwrap();
        assert_eq!(command.program, "codewhale");
        assert_eq!(
            command.args,
            vec!["--resume", session.agent_session_id.as_str()]
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn falls_back_to_first_user_message_for_title() {
        let root = temp_root("title");
        let session = root.join("aaaaaaaa-1111-2222-3333-444444444444.json");
        std::fs::write(
            &session,
            r#"{
  "schema_version": 1,
  "metadata": {
    "id": "aaaaaaaa-1111-2222-3333-444444444444",
    "title": "",
    "created_at": "2026-08-03T11:06:26.000000000Z",
    "workspace": "/data/proj"
  },
  "messages": [
    {"role": "user", "content": [{"type": "text", "text": "中文搜索功能"}]}
  ]
}"#,
        )
        .unwrap();

        let sessions = CodewhaleAdapter::new(root.clone()).list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "中文搜索功能");

        std::fs::remove_dir_all(&root).ok();
    }
}
