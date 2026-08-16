use std::path::PathBuf;

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use zellij_ai_session_core::{
    AgentKind, AiSession, CommandSpec, SessionStatus, project_for_directory,
};

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
            let project = project_for_directory(std::path::Path::new(&directory));
            sessions.push(AiSession {
                id: format!("opencode:{id}"),
                agent: AgentKind::OpenCode,
                title: if title.trim().is_empty() {
                    "Untitled session".into()
                } else {
                    title
                },
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
