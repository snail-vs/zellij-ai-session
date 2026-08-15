use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use zellij_ai_session_core::{
    AgentKind, AiSession, CommandSpec, SessionStatus, project_for_directory,
};

use crate::adapters::AgentAdapter;

const SEARCH_TEXT_LIMIT: usize = 64 * 1024;

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

        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        drop(statement);
        let search_text = read_search_text(&connection)?;

        let mut sessions = Vec::new();
        for (id, directory, title, created_at_ms, updated_at_ms) in records {
            let project = project_for_directory(std::path::Path::new(&directory));
            sessions.push(AiSession {
                id: format!("opencode:{id}"),
                agent: AgentKind::OpenCode,
                title: if title.trim().is_empty() {
                    "Untitled session".into()
                } else {
                    title
                },
                search_text: search_text.get(&id).cloned().unwrap_or_default(),
                project_id: project.id,
                directory: PathBuf::from(directory),
                created_at_ms: Some(created_at_ms),
                updated_at_ms: Some(updated_at_ms),
                agent_session_id: id,
                status: SessionStatus::Historical,
                runtime: None,
            });
        }
        Ok(sessions)
    }

    fn resume_command(&self, session: &AiSession) -> Result<CommandSpec> {
        Ok(CommandSpec::new("opencode", session.directory.clone())
            .with_args(["--session", session.agent_session_id.as_str()]))
    }
}

fn read_search_text(connection: &Connection) -> Result<HashMap<String, String>> {
    let mut statement = connection.prepare(
        "SELECT p.session_id, m.data, p.data
         FROM part p
         JOIN message m ON m.id = p.message_id
         ORDER BY p.time_created",
    )?;
    let rows = statement.query_map([], |row| {
        let session_id: String = row.get(0)?;
        let message: String = row.get(1)?;
        let part: String = row.get(2)?;
        Ok((session_id, message, part))
    })?;

    let mut search_text = HashMap::new();
    for row in rows {
        let (session_id, message, part) = row?;
        let is_searchable_message = serde_json::from_str::<serde_json::Value>(&message)
            .ok()
            .and_then(|value| {
                value
                    .get("role")
                    .and_then(serde_json::Value::as_str)
                    .map(|role| matches!(role, "user" | "assistant"))
            })
            .unwrap_or(false);
        if !is_searchable_message {
            continue;
        }
        let Ok(part) = serde_json::from_str::<serde_json::Value>(&part) else {
            continue;
        };
        let text = search_text.entry(session_id).or_default();
        append_part_text(text, &part);
    }
    Ok(search_text)
}

fn append_part_text(output: &mut String, part: &serde_json::Value) {
    append_json_text(output, part.get("text"));
    append_json_text(output, part.pointer("/state/output"));
    append_json_text(output, part.pointer("/state/error"));
    append_json_text(output, part.pointer("/state/title"));
    append_json_text(output, part.get("prompt"));
    append_json_text(output, part.get("description"));
}

fn append_json_text(output: &mut String, value: Option<&serde_json::Value>) {
    let Some(serde_json::Value::String(text)) = value else {
        return;
    };
    let text = text.trim();
    if text.is_empty() || output.len() >= SEARCH_TEXT_LIMIT {
        return;
    }
    if !output.is_empty() {
        output.push('\n');
    }
    let remaining = SEARCH_TEXT_LIMIT.saturating_sub(output.len());
    let end = text
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(text.len()))
        .take_while(|index| *index <= remaining)
        .last()
        .unwrap_or_default();
    output.push_str(&text[..end]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opencode_part_keeps_user_content_for_search() {
        let part = serde_json::json!({
            "type": "text",
            "text": "查找中文会话"
        });
        let mut search_text = String::new();
        append_part_text(&mut search_text, &part);
        assert!(search_text.contains("中文会话"));
    }
}
