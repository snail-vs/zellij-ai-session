use std::path::PathBuf;

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use zellij_ai_session_core::{AgentKind, AiSession, CommandSpec, PreviewMessage, SessionPreview};

use crate::adapters::AgentAdapter;

pub struct OpenCodeAdapter {
    database: PathBuf,
}

impl OpenCodeAdapter {
    pub fn new(database: PathBuf) -> Self {
        Self { database }
    }
}

impl AgentAdapter for OpenCodeAdapter {
    fn name(&self) -> &'static str {
        "OpenCode"
    }
    fn agent(&self) -> AgentKind {
        AgentKind::OpenCode
    }

    fn rename_session(&self, session: &AiSession, title: &str) -> Result<()> {
        let default_database = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
            .context("cannot locate OpenCode's default data directory")?
            .join("opencode/opencode.db");
        anyhow::ensure!(
            self.database == default_database,
            "native renaming is unavailable for a custom OpenCode database"
        );
        crate::native_rename::rename_opencode(&session.agent_session_id, title)
    }

    fn list_sessions(&self) -> Result<Vec<AiSession>> {
        if !self.database.exists() {
            return Ok(Vec::new());
        }
        let connection =
            Connection::open_with_flags(&self.database, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .with_context(|| format!("open OpenCode database {}", self.database.display()))?;
        let mut statement = connection.prepare(
            "SELECT id, directory, title, time_created, time_updated FROM session ORDER BY time_updated DESC",
        )?;
        let rows = statement.query_map([], |row| {
            let id: String = row.get(0)?;
            let directory: String = row.get(1)?;
            let title: String = row.get(2)?;
            let created_at_ms: i64 = row.get(3)?;
            let updated_at_ms: i64 = row.get(4)?;
            Ok((id, directory, title, created_at_ms, updated_at_ms))
        })?;

        let mut sessions = Vec::new();
        for row in rows {
            let (id, directory, title, created_at_ms, updated_at_ms) = row?;
            let title = if title.trim().is_empty() {
                "Untitled session".into()
            } else {
                title
            };
            sessions.push(AiSession::new(
                AgentKind::OpenCode,
                &id,
                title,
                PathBuf::from(directory),
                Some(created_at_ms),
                Some(updated_at_ms),
            ));
        }
        Ok(sessions)
    }

    fn resume_command(&self, session: &AiSession) -> Result<CommandSpec> {
        Ok(CommandSpec::new("opencode", session.directory.clone())
            .with_args(["--session", session.agent_session_id.as_str()]))
    }

    fn preview(&self, session_id: &str) -> Result<SessionPreview> {
        if !self.database.exists() {
            anyhow::bail!("OpenCode database is unavailable");
        }
        let connection =
            Connection::open_with_flags(&self.database, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM session WHERE id = ?1)",
            [session_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(SessionPreview {
                session_id: session_id.into(),
                messages: Vec::new(),
                note: Some("Native OpenCode session was not found".into()),
            });
        }
        let mut statement = connection.prepare(
            "SELECT m.data, p.data FROM message m LEFT JOIN part p ON p.message_id = m.id AND p.session_id = m.session_id WHERE m.session_id = ?1 ORDER BY m.time_created DESC, m.id DESC, p.time_created DESC LIMIT 100"
        )?;
        let mut rows = statement.query([session_id])?;
        let mut messages = Vec::new();
        while let Some(row) = rows.next()? {
            let message: String = row.get(0)?;
            let part: Option<String> = row.get(1)?;
            let Ok(message) = serde_json::from_str::<serde_json::Value>(&message) else {
                continue;
            };
            let Some(role) = message.get("role").and_then(serde_json::Value::as_str) else {
                continue;
            };
            if !matches!(role, "user" | "assistant") {
                continue;
            }
            let Some(part) =
                part.and_then(|part| serde_json::from_str::<serde_json::Value>(&part).ok())
            else {
                continue;
            };
            if part.get("type").and_then(serde_json::Value::as_str) != Some("text") {
                continue;
            }
            let Some(text) = part.get("text").and_then(serde_json::Value::as_str) else {
                continue;
            };
            if text.trim().is_empty() {
                continue;
            }
            messages.push(PreviewMessage {
                role: role.into(),
                text: text.into(),
            });
            if messages.len() == 8 {
                break;
            }
        }
        messages.reverse();
        Ok(SessionPreview {
            session_id: session_id.into(),
            note: messages
                .is_empty()
                .then(|| "No readable user or Agent messages have been saved yet".into()),
            messages,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_uses_message_and_part_session_id() {
        let path = std::env::temp_dir().join(format!("opencode-preview-{}.db", std::process::id()));
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE session (id TEXT); CREATE TABLE message (id TEXT, session_id TEXT, time_created INTEGER, data TEXT); CREATE TABLE part (id TEXT, message_id TEXT, session_id TEXT, time_created INTEGER, data TEXT); INSERT INTO session VALUES ('one'), ('two'); INSERT INTO message VALUES ('m1','one',1,'{\"role\":\"user\"}'),('m2','two',2,'{\"role\":\"assistant\"}'); INSERT INTO part VALUES ('p1','m1','one',1,'{\"type\":\"text\",\"text\":\"first\"}'),('p2','m2','two',2,'{\"type\":\"text\",\"text\":\"second\"}');").unwrap();
        drop(connection);
        let preview = OpenCodeAdapter::new(path.clone()).preview("two").unwrap();
        assert_eq!(preview.messages.len(), 1);
        assert_eq!(preview.messages[0].text, "second");
        std::fs::remove_file(path).unwrap();
    }
}
