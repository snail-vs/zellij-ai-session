use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags, params};
use serde_json::Value;
use zellij_ai_session_core::{AgentKind, AiSession, CommandSpec, PreviewMessage, SessionPreview};

use crate::adapters::AgentAdapter;

/// Rows pulled from the message store while building one preview.
const PREVIEW_ROW_LIMIT: i64 = 100;
/// Preview messages kept for a session, newest ones win.
const PREVIEW_MESSAGE_LIMIT: usize = 8;

/// OpenCode has shipped two generations of SQLite layout.
///
/// 1.x keeps sessions in `session` and splits their transcript into `message`
/// plus one `part` row per content block. 2.0 projects the same history into
/// `session_v2` and a single `session_message` table whose `data` column inlines
/// the content blocks, and writes newly created sessions only there.
///
/// A migrated 2.0 database keeps both generations around, so both session tables
/// are read and merged: `session_v2` is read first and wins when the same session
/// appears twice. Reading both tables also covers a downgraded install, where
/// 1.x appends to `session` while `session_v2` still holds the rows that 2.0
/// created before the downgrade.
#[derive(Debug, Clone)]
struct Schema {
    /// Session tables present in this database, most authoritative first.
    session_tables: Vec<&'static str>,
    /// Whether `session_message` is available as the transcript source.
    projected_messages: bool,
}

impl Schema {
    fn detect(connection: &Connection) -> Result<Self> {
        let mut session_tables = Vec::new();
        if has_table(connection, "session_v2")? {
            session_tables.push("session_v2");
        }
        if has_table(connection, "session")? {
            session_tables.push("session");
        }
        Ok(Self {
            session_tables,
            projected_messages: has_table(connection, "session_message")?,
        })
    }

    /// Whether the session exists in any of the known session tables.
    fn contains(&self, connection: &Connection, session_id: &str) -> Result<bool> {
        for table in &self.session_tables {
            let found: bool = connection.query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE id = ?1)"),
                [session_id],
                |row| row.get(0),
            )?;
            if found {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

pub struct OpenCodeAdapter {
    database: PathBuf,
}

impl OpenCodeAdapter {
    pub fn new(database: PathBuf) -> Self {
        Self { database }
    }

    fn connect(&self) -> Result<Connection> {
        Connection::open_with_flags(&self.database, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("open OpenCode database {}", self.database.display()))
    }

    fn read_sessions(&self, connection: &Connection, table: &str) -> Result<Vec<AiSession>> {
        let sql = format!(
            "SELECT id, directory, title, time_created, time_updated FROM {table} ORDER BY time_updated DESC"
        );
        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map([], |row| {
            let id: String = row.get(0)?;
            let directory: String = row.get(1)?;
            // `session_v2` declares `title` nullable, `session` does not.
            let title: Option<String> = row.get(2)?;
            let created_at_ms: i64 = row.get(3)?;
            let updated_at_ms: i64 = row.get(4)?;
            Ok((id, directory, title, created_at_ms, updated_at_ms))
        })?;

        let mut sessions = Vec::new();
        for row in rows {
            let (id, directory, title, created_at_ms, updated_at_ms) = row?;
            let title = match title {
                Some(title) if !title.trim().is_empty() => title,
                _ => "Untitled session".into(),
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

    /// 2.0 transcript: one `session_message` row per message, with the content
    /// blocks inlined in `data`. A user message carries its prompt in `text`,
    /// an assistant message carries blocks in `content`. Rows come back newest
    /// first so the preview keeps the tail of the conversation.
    fn projected_preview(
        &self,
        connection: &Connection,
        session_id: &str,
    ) -> Result<Vec<PreviewMessage>> {
        let mut statement = connection.prepare(
            "SELECT type, data FROM session_message WHERE session_id = ?1 ORDER BY seq DESC LIMIT ?2",
        )?;
        let mut rows = statement.query(params![session_id, PREVIEW_ROW_LIMIT])?;
        let mut messages = Vec::new();
        while let Some(row) = rows.next()? {
            let role: String = row.get(0)?;
            if !matches!(role.as_str(), "user" | "assistant") {
                continue;
            }
            let data: String = row.get(1)?;
            let Ok(data) = serde_json::from_str::<Value>(&data) else {
                continue;
            };
            let texts: Vec<&str> = match data.get("content") {
                Some(Value::Array(blocks)) => blocks
                    .iter()
                    .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|block| block.get("text").and_then(Value::as_str))
                    .collect(),
                _ => data
                    .get("text")
                    .and_then(Value::as_str)
                    .into_iter()
                    .collect(),
            };
            for text in texts {
                if text.trim().is_empty() {
                    continue;
                }
                messages.push(PreviewMessage {
                    role: role.clone(),
                    text: text.into(),
                });
            }
        }
        Ok(tail(messages))
    }

    /// 1.x transcript: message metadata in `message.data`, one content block per
    /// `part` row joined back through `message_id` and `session_id`.
    fn legacy_preview(
        &self,
        connection: &Connection,
        session_id: &str,
    ) -> Result<Vec<PreviewMessage>> {
        let mut statement = connection.prepare(
            "SELECT m.data, p.data FROM message m LEFT JOIN part p ON p.message_id = m.id AND p.session_id = m.session_id WHERE m.session_id = ?1 ORDER BY m.time_created DESC, m.id DESC, p.time_created DESC LIMIT ?2",
        )?;
        let mut rows = statement.query(params![session_id, PREVIEW_ROW_LIMIT])?;
        let mut messages = Vec::new();
        while let Some(row) = rows.next()? {
            let message: String = row.get(0)?;
            let part: Option<String> = row.get(1)?;
            let Ok(message) = serde_json::from_str::<Value>(&message) else {
                continue;
            };
            let Some(role) = message.get("role").and_then(Value::as_str) else {
                continue;
            };
            if !matches!(role, "user" | "assistant") {
                continue;
            }
            let Some(part) = part.and_then(|part| serde_json::from_str::<Value>(&part).ok()) else {
                continue;
            };
            if part.get("type").and_then(Value::as_str) != Some("text") {
                continue;
            }
            let Some(text) = part.get("text").and_then(Value::as_str) else {
                continue;
            };
            if text.trim().is_empty() {
                continue;
            }
            messages.push(PreviewMessage {
                role: role.into(),
                text: text.into(),
            });
            if messages.len() == PREVIEW_MESSAGE_LIMIT {
                break;
            }
        }
        Ok(tail(messages))
    }
}

/// Turn newest-first rows into the last `PREVIEW_MESSAGE_LIMIT` messages in
/// chronological order.
fn tail(mut messages: Vec<PreviewMessage>) -> Vec<PreviewMessage> {
    messages.reverse();
    let excess = messages.len().saturating_sub(PREVIEW_MESSAGE_LIMIT);
    messages.drain(..excess);
    messages
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
        let connection = self.connect()?;
        let schema = Schema::detect(&connection)?;
        let mut sessions = Vec::new();
        let mut seen = HashSet::new();
        for table in &schema.session_tables {
            for session in self.read_sessions(&connection, table)? {
                // Both tables can carry the same session after a migration; the
                // newer copy is read first and wins.
                if seen.insert(session.agent_session_id.clone()) {
                    sessions.push(session);
                }
            }
        }
        sessions.sort_by_key(|session| std::cmp::Reverse(session.updated_at_ms));
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
        let connection = self.connect()?;
        let schema = Schema::detect(&connection)?;
        if !schema.contains(&connection, session_id)? {
            return Ok(SessionPreview {
                session_id: session_id.into(),
                messages: Vec::new(),
                note: Some("Native OpenCode session was not found".into()),
            });
        }
        let mut messages = Vec::new();
        if schema.projected_messages {
            messages = self.projected_preview(&connection, session_id)?;
        }
        if messages.is_empty() && has_table(&connection, "message")? {
            messages = self.legacy_preview(&connection, session_id)?;
        }
        Ok(SessionPreview {
            session_id: session_id.into(),
            note: messages
                .is_empty()
                .then(|| "No readable user or Agent messages have been saved yet".into()),
            messages,
        })
    }
}

fn has_table(connection: &Connection, table: &str) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get(0),
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("opencode-{name}-{}.db", std::process::id()))
    }

    fn seed(name: &str, schema: &str) -> PathBuf {
        let path = temp_path(name);
        let _ = std::fs::remove_file(&path);
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(schema).unwrap();
        drop(connection);
        path
    }

    const LEGACY_SCHEMA: &str = "
        CREATE TABLE session (id TEXT PRIMARY KEY, directory TEXT NOT NULL, title TEXT NOT NULL, time_created INTEGER, time_updated INTEGER);
        CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, time_created INTEGER, data TEXT);
        CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT NOT NULL, session_id TEXT NOT NULL, time_created INTEGER, data TEXT);
        INSERT INTO session VALUES ('one', '/tmp/legacy', 'legacy session', 1, 10), ('two', '/tmp/legacy', 'other session', 3, 40);
        INSERT INTO message VALUES ('m1', 'one', 1, '{\"role\":\"user\"}'), ('m2', 'one', 2, '{\"role\":\"assistant\"}'), ('m3', 'two', 3, '{\"role\":\"assistant\"}');
        INSERT INTO part VALUES ('p1', 'm1', 'one', 1, '{\"type\":\"text\",\"text\":\"first\"}'), ('p2', 'm2', 'one', 2, '{\"type\":\"text\",\"text\":\"second\"}'), ('p3', 'm3', 'two', 3, '{\"type\":\"text\",\"text\":\"second\"}');
    ";

    const PROJECTED_SCHEMA: &str = "
        CREATE TABLE session_v2 (id TEXT PRIMARY KEY, directory TEXT NOT NULL, title TEXT, time_created INTEGER, time_updated INTEGER);
        CREATE TABLE session_message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, type TEXT NOT NULL, seq INTEGER NOT NULL, time_created INTEGER, data TEXT);
        INSERT INTO session_v2 VALUES ('one', '/tmp/modern', 'modern session', 1, 20), ('untitled', '/tmp/modern', NULL, 2, 30);
        INSERT INTO session_message VALUES
            ('s1', 'one', 'user', 1, 1, '{\"text\":\"first\"}'),
            ('s2', 'one', 'assistant', 2, 2, '{\"content\":[{\"type\":\"reasoning\",\"text\":\"hidden\"},{\"type\":\"text\",\"text\":\"second\"},{\"type\":\"tool\",\"name\":\"read\"}]}'),
            ('s3', 'one', 'idle', 3, 3, '{}');
    ";

    #[test]
    fn preview_uses_message_and_part_session_id() {
        let path = seed("preview", LEGACY_SCHEMA);
        // `two` only owns `m3`; the join must not leak `one`'s parts.
        let preview = OpenCodeAdapter::new(path.clone()).preview("two").unwrap();
        assert_eq!(preview.messages.len(), 1);
        assert_eq!(preview.messages[0].text, "second");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn legacy_database_still_lists_and_previews_sessions() {
        let path = seed("legacy", LEGACY_SCHEMA);
        let adapter = OpenCodeAdapter::new(path.clone());
        let sessions = adapter.list_sessions().unwrap();
        assert_eq!(sessions.len(), 2);
        // Newest `time_updated` first.
        assert_eq!(sessions[0].agent_session_id, "two");
        assert_eq!(sessions[1].agent_session_id, "one");
        assert_eq!(sessions[1].title, "legacy session");
        let preview = adapter.preview("one").unwrap();
        assert_eq!(preview.messages.len(), 2);
        assert_eq!(preview.messages[0].role, "user");
        assert_eq!(preview.messages[1].text, "second");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn projected_database_lists_and_previews_sessions() {
        let path = seed("projected", PROJECTED_SCHEMA);
        let adapter = OpenCodeAdapter::new(path.clone());
        let sessions = adapter.list_sessions().unwrap();
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].agent_session_id, "untitled");
        assert_eq!(sessions[0].title, "Untitled session");
        assert_eq!(sessions[1].title, "modern session");
        let preview = adapter.preview("one").unwrap();
        assert_eq!(preview.messages.len(), 2);
        assert_eq!(preview.messages[0].text, "first");
        // Reasoning and tool blocks stay out of the preview.
        assert_eq!(preview.messages[1].role, "assistant");
        assert_eq!(preview.messages[1].text, "second");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn migrated_database_merges_both_tables_without_duplicates() {
        let path = seed("migrated", &format!("{LEGACY_SCHEMA}{PROJECTED_SCHEMA}"));
        let adapter = OpenCodeAdapter::new(path.clone());
        let sessions = adapter.list_sessions().unwrap();
        // `one` exists in both tables but must be listed once, from `session_v2`.
        // `two` only exists in the 1.x table (as after a downgrade) and `untitled`
        // only in the 2.0 one, so both survive.
        assert_eq!(sessions.len(), 3);
        assert_eq!(
            sessions
                .iter()
                .map(|s| s.agent_session_id.as_str())
                .collect::<Vec<_>>(),
            ["two", "untitled", "one"]
        );
        assert_eq!(sessions[2].title, "modern session");
        assert_eq!(sessions[2].directory, Path::new("/tmp/modern"));
        let preview = adapter.preview("one").unwrap();
        assert_eq!(preview.messages[0].text, "first");
        assert_eq!(preview.messages[1].text, "second");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn preview_falls_back_to_legacy_tables_when_projection_is_empty() {
        let path = seed(
            "fallback",
            &format!(
                "{LEGACY_SCHEMA}
                 CREATE TABLE session_v2 (id TEXT PRIMARY KEY, directory TEXT NOT NULL, title TEXT, time_created INTEGER, time_updated INTEGER);
                 CREATE TABLE session_message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, type TEXT NOT NULL, seq INTEGER NOT NULL, time_created INTEGER, data TEXT);"
            ),
        );
        let adapter = OpenCodeAdapter::new(path.clone());
        let preview = adapter.preview("one").unwrap();
        assert_eq!(preview.messages.len(), 2);
        assert_eq!(preview.messages[0].text, "first");
        assert_eq!(preview.messages[1].text, "second");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn legacy_sessions_are_listed_when_the_projected_table_is_empty() {
        let path = seed(
            "half-migrated",
            &format!(
                "{LEGACY_SCHEMA}
                 CREATE TABLE session_v2 (id TEXT PRIMARY KEY, directory TEXT NOT NULL, title TEXT, time_created INTEGER, time_updated INTEGER);
                 CREATE TABLE session_message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, type TEXT NOT NULL, seq INTEGER NOT NULL, time_created INTEGER, data TEXT);"
            ),
        );
        let sessions = OpenCodeAdapter::new(path.clone()).list_sessions().unwrap();
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[1].agent_session_id, "one");
        assert_eq!(sessions[1].title, "legacy session");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn missing_session_reports_that_it_was_not_found() {
        let path = seed("missing", PROJECTED_SCHEMA);
        let preview = OpenCodeAdapter::new(path.clone()).preview("nope").unwrap();
        assert!(preview.messages.is_empty());
        assert_eq!(
            preview.note.as_deref(),
            Some("Native OpenCode session was not found")
        );
        std::fs::remove_file(path).unwrap();
    }
}
