use std::path::PathBuf;

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use zellij_ai_session_core::{AgentKind, AiSession, CommandSpec};

use crate::adapters::{AgentAdapter, first_text, iso_to_ms};

pub struct GooseAdapter {
    database: PathBuf,
}

impl GooseAdapter {
    pub fn new(database: PathBuf) -> Self {
        Self { database }
    }
}

/// Parse a SQLite `TIMESTAMP` value (ISO 8601 or `YYYY-MM-DD HH:MM:SS`) into
/// epoch milliseconds.
fn parse_time(value: String) -> Option<i64> {
    if let Some(ms) = iso_to_ms(&value) {
        return Some(ms);
    }
    chrono::NaiveDateTime::parse_from_str(&value, "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|datetime| datetime.and_utc().timestamp_millis())
}

impl AgentAdapter for GooseAdapter {
    fn name(&self) -> &'static str {
        "Goose"
    }

    fn agent(&self) -> AgentKind {
        AgentKind::Goose
    }

    fn list_sessions(&self) -> Result<Vec<AiSession>> {
        if !self.database.exists() {
            return Ok(Vec::new());
        }
        let connection =
            Connection::open_with_flags(&self.database, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .with_context(|| format!("open Goose database {}", self.database.display()))?;

        let mut statement = connection.prepare(
            "SELECT id, working_dir, name, created_at, updated_at FROM sessions ORDER BY updated_at DESC",
        )?;
        let rows = statement.query_map([], |row| {
            let id: String = row.get(0)?;
            let working_dir: String = row.get(1)?;
            let name: String = row.get(2)?;
            let created_at: String = row.get(3)?;
            let updated_at: String = row.get(4)?;
            Ok((id, working_dir, name, created_at, updated_at))
        })?;

        let mut sessions = Vec::new();
        for row in rows {
            let (id, working_dir, name, created_at, updated_at) = row?;
            if working_dir.trim().is_empty() {
                continue;
            }

            let title = if name.trim().is_empty() {
                first_user_message(&connection, &id).unwrap_or_else(|| "Untitled session".into())
            } else {
                name
            };

            sessions.push(AiSession::new(
                AgentKind::Goose,
                &id,
                title,
                PathBuf::from(working_dir),
                parse_time(created_at),
                parse_time(updated_at),
            ));
        }
        Ok(sessions)
    }

    fn resume_command(&self, session: &AiSession) -> Result<CommandSpec> {
        Ok(
            CommandSpec::new("goose", session.directory.clone()).with_args([
                "session",
                "--resume",
                "--session-id",
                session.agent_session_id.as_str(),
            ]),
        )
    }
}

/// First user message text for a session, used as a fallback title.
fn first_user_message(connection: &Connection, session_id: &str) -> Option<String> {
    let mut statement = connection
        .prepare(
            "SELECT content FROM messages WHERE session_id = ? AND role = 'user' ORDER BY timestamp ASC LIMIT 1",
        )
        .ok()?;
    let mut rows = statement.query([session_id]).ok()?;
    let content: String = rows.next().ok()??.get(0).ok()?;
    let value: Value = serde_json::from_str(&content).ok()?;
    first_text(&value).map(|text| {
        text.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(200)
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sessions_from_sqlite() {
        let dir = std::env::temp_dir().join(format!(
            "zais-goose-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let sessions_dir = dir.join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let db = sessions_dir.join("sessions.db");

        let connection = Connection::open(&db).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE sessions (id TEXT PRIMARY KEY, working_dir TEXT NOT NULL, name TEXT, created_at TIMESTAMP, updated_at TIMESTAMP);
                 CREATE TABLE messages (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL, timestamp TIMESTAMP);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO sessions (id, working_dir, name, created_at, updated_at) VALUES ('20250710_1', '/data/proj', '', '2025-07-10 09:00:00', '2025-07-10 09:30:00')",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO sessions (id, working_dir, name, created_at, updated_at) VALUES ('20250711_1', '/data/other', 'My named session', '2025-07-11 10:00:00', '2025-07-11 10:05:00')",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO messages (id, session_id, role, content, timestamp) VALUES ('m1', '20250710_1', 'user', '{\"text\":\"中文搜索怎么修\"}', '2025-07-10 09:01:00')",
                [],
            )
            .unwrap();

        let sessions = GooseAdapter::new(db.clone()).list_sessions().unwrap();
        assert_eq!(sessions.len(), 2);

        let unnamed = sessions
            .iter()
            .find(|s| s.id.contains("20250710_1"))
            .unwrap();
        assert_eq!(unnamed.title, "中文搜索怎么修");
        assert_eq!(unnamed.directory, PathBuf::from("/data/proj"));
        assert_eq!(unnamed.created_at_ms, Some(1_752_138_000_000));
        assert_eq!(unnamed.updated_at_ms, Some(1_752_139_800_000));

        let named = sessions
            .iter()
            .find(|s| s.id.contains("20250711_1"))
            .unwrap();
        assert_eq!(named.title, "My named session");

        let command = GooseAdapter::new(db).resume_command(unnamed).unwrap();
        assert_eq!(command.program, "goose");
        assert_eq!(
            command.args,
            vec!["session", "--resume", "--session-id", "20250710_1"]
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
