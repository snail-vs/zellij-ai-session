//! Sparse workbench metadata. Native conversation history stays in each CLI store.
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use zellij_ai_session_core::{
    AgentKind, AiSession, IndexSnapshot, Project, ProjectOrigin, build_snapshot_with_projects,
    normalize_directory,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    pub id: String,
    pub project_id: String,
    pub parent_id: Option<String>,
    pub title_override: Option<String>,
    pub selected_cwd: Option<PathBuf>,
    pub handoff_text: Option<String>,
    pub handoff_path: Option<PathBuf>,
    pub work_state: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingLaunch {
    pub request_id: String,
    pub parent_id: Option<String>,
    pub project_id: String,
    pub tool: AgentKind,
    pub title: String,
    pub selected_cwd: PathBuf,
    pub handoff_text: Option<String>,
    pub handoff_path: Option<PathBuf>,
    pub state: String,
    pub started_at_ms: Option<i64>,
    pub observed_pane: Option<String>,
}

pub struct WorkbenchStore {
    connection: Connection,
}

impl WorkbenchStore {
    pub fn default_path() -> Result<PathBuf> {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
            .context("HOME or XDG_DATA_HOME is required for the workbench database")?;
        Ok(base.join("zellij-ai-session/workbench.db"))
    }

    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        let connection =
            Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > 1 {
            bail!("workbench database schema {version} is newer than supported schema 1");
        }
        if version == 0 {
            connection.execute_batch(
                "BEGIN IMMEDIATE;
                CREATE TABLE IF NOT EXISTS project (
                    id TEXT PRIMARY KEY, name TEXT NOT NULL, root_directory TEXT NOT NULL UNIQUE,
                    origin TEXT NOT NULL CHECK(origin IN ('auto', 'manual'))
                );
                CREATE TABLE IF NOT EXISTS session (
                    id TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES project(id),
                    parent_id TEXT REFERENCES session(id), title_override TEXT, selected_cwd TEXT,
                    handoff_text TEXT, handoff_path TEXT, work_state TEXT,
                    created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL,
                    CHECK(parent_id IS NULL OR parent_id <> id)
                );
                CREATE INDEX IF NOT EXISTS session_project_idx ON session(project_id);
                CREATE TABLE IF NOT EXISTS pending_launch (
                    request_id TEXT PRIMARY KEY, parent_id TEXT REFERENCES session(id),
                    project_id TEXT NOT NULL REFERENCES project(id), tool TEXT NOT NULL,
                    title TEXT NOT NULL, selected_cwd TEXT NOT NULL, handoff_text TEXT,
                    handoff_path TEXT, state TEXT NOT NULL, started_at_ms INTEGER,
                    observed_pane TEXT
                );
                PRAGMA user_version = 1; COMMIT;",
            )?;
        }
        Ok(Self { connection })
    }

    pub fn create_project(
        &mut self,
        name: &str,
        root: &Path,
        origin: ProjectOrigin,
    ) -> Result<Project> {
        if name.trim().is_empty() {
            bail!("project name cannot be empty");
        }
        let root = normalize_directory(root);
        let root_text = root.to_string_lossy().into_owned();
        if let Some(mut project) = self.project_by_root(&root)? {
            if origin == ProjectOrigin::Manual && project.origin == ProjectOrigin::Auto {
                self.connection.execute(
                    "UPDATE project SET name=?2, origin='manual' WHERE id=?1",
                    params![project.id, name],
                )?;
                project.name = name.to_owned();
                project.origin = ProjectOrigin::Manual;
            }
            return Ok(project);
        }
        let id: String =
            self.connection
                .query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))?;
        let origin_text = match origin {
            ProjectOrigin::Auto => "auto",
            ProjectOrigin::Manual => "manual",
        };
        self.connection.execute(
            "INSERT INTO project VALUES (?1, ?2, ?3, ?4)",
            params![id, name, root_text, origin_text],
        )?;
        Ok(Project {
            id,
            name: name.to_owned(),
            root_directory: root,
            origin,
        })
    }

    fn project_by_root(&self, root: &Path) -> Result<Option<Project>> {
        let mut statement = self.connection.prepare(
            "SELECT id, name, root_directory, origin FROM project WHERE root_directory = ?1",
        )?;
        Ok(statement
            .query_row([root.to_string_lossy().as_ref()], project_row)
            .optional()?)
    }

    pub fn projects(&self) -> Result<Vec<Project>> {
        let mut statement = self
            .connection
            .prepare("SELECT id, name, root_directory, origin FROM project")?;
        Ok(statement
            .query_map([], project_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn rename_project(&self, id: &str, name: &str) -> Result<()> {
        if name.trim().is_empty() {
            bail!("project name cannot be empty");
        }
        if self
            .connection
            .execute("UPDATE project SET name=?2 WHERE id=?1", params![id, name])?
            == 0
        {
            bail!("project {id} does not exist");
        }
        Ok(())
    }

    /// Drop a legacy local title so subsequent scans display the native name.
    pub fn clear_title_override(&self, id: &str) -> Result<()> {
        self.connection
            .execute("UPDATE session SET title_override=NULL WHERE id=?1", [id])?;
        Ok(())
    }

    pub fn save_session(&mut self, record: &SessionRecord) -> Result<()> {
        parse_key(&record.id)?;
        let transaction = self.connection.transaction()?;
        let old_project: Option<String> = transaction
            .query_row(
                "SELECT project_id FROM session WHERE id=?1",
                [&record.id],
                |row| row.get(0),
            )
            .optional()?;
        if old_project
            .as_deref()
            .is_some_and(|old| old != record.project_id)
        {
            bail!("moving a saved session between projects is not supported");
        }
        validate_parent(
            &transaction,
            &record.id,
            &record.project_id,
            record.parent_id.as_deref(),
        )?;
        transaction.execute("INSERT INTO session
            (id, project_id, parent_id, title_override, selected_cwd, handoff_text, handoff_path,
             work_state, created_at_ms, updated_at_ms)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            ON CONFLICT(id) DO UPDATE SET project_id=excluded.project_id, parent_id=excluded.parent_id,
             title_override=excluded.title_override, selected_cwd=excluded.selected_cwd,
             handoff_text=excluded.handoff_text, handoff_path=excluded.handoff_path,
             work_state=excluded.work_state, updated_at_ms=excluded.updated_at_ms",
            params![record.id, record.project_id, record.parent_id, record.title_override,
                path_text(record.selected_cwd.as_deref()), record.handoff_text,
                path_text(record.handoff_path.as_deref()), record.work_state,
                record.created_at_ms, record.updated_at_ms])?;
        transaction.commit()?;
        Ok(())
    }

    /// Materialize a scanned parent or child only when metadata must be saved.
    pub fn ensure_session(&mut self, session: &AiSession, project_id: &str) -> Result<()> {
        if self.session(&session.id)?.is_some() {
            return Ok(());
        }
        self.save_session(&SessionRecord {
            id: session.id.clone(),
            project_id: project_id.into(),
            parent_id: None,
            title_override: None,
            selected_cwd: None,
            handoff_text: None,
            handoff_path: None,
            work_state: None,
            created_at_ms: session.created_at_ms.unwrap_or(0),
            updated_at_ms: session.updated_at_ms.unwrap_or(0),
        })
    }

    pub fn session(&self, id: &str) -> Result<Option<SessionRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT id, project_id, parent_id, title_override,
            selected_cwd, handoff_text, handoff_path, work_state, created_at_ms, updated_at_ms
            FROM session WHERE id = ?1",
        )?;
        Ok(statement.query_row([id], session_row).optional()?)
    }

    pub fn sessions(&self) -> Result<Vec<SessionRecord>> {
        let mut statement = self.connection.prepare("SELECT id, project_id, parent_id, title_override,
            selected_cwd, handoff_text, handoff_path, work_state, created_at_ms, updated_at_ms FROM session")?;
        Ok(statement
            .query_map([], session_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn set_parent(&mut self, id: &str, parent_id: Option<&str>) -> Result<()> {
        let transaction = self.connection.transaction()?;
        let project_id: String = transaction
            .query_row("SELECT project_id FROM session WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .with_context(|| format!("session {id} must be saved before setting its parent"))?;
        validate_parent(&transaction, id, &project_id, parent_id)?;
        transaction.execute(
            "UPDATE session SET parent_id=?2 WHERE id=?1",
            params![id, parent_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn save_pending(&self, pending: &PendingLaunch) -> Result<()> {
        validate_parent(
            &self.connection,
            "",
            &pending.project_id,
            pending.parent_id.as_deref(),
        )?;
        self.connection.execute(
            "INSERT INTO pending_launch VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            ON CONFLICT(request_id) DO UPDATE SET state=excluded.state,
             started_at_ms=excluded.started_at_ms, observed_pane=excluded.observed_pane",
            params![
                pending.request_id,
                pending.parent_id,
                pending.project_id,
                pending.tool.command_name(),
                pending.title,
                pending.selected_cwd.to_string_lossy(),
                pending.handoff_text,
                path_text(pending.handoff_path.as_deref()),
                pending.state,
                pending.started_at_ms,
                pending.observed_pane
            ],
        )?;
        Ok(())
    }

    pub fn pending(&self) -> Result<Vec<PendingLaunch>> {
        let mut statement = self.connection.prepare(
            "SELECT request_id, parent_id, project_id, tool,
            title, selected_cwd, handoff_text, handoff_path, state, started_at_ms, observed_pane
            FROM pending_launch",
        )?;
        Ok(statement
            .query_map([], |row| {
                let tool: String = row.get(3)?;
                let tool = AgentKind::from_command_name(&tool).ok_or_else(|| {
                    rusqlite::Error::InvalidColumnType(
                        3,
                        "tool".into(),
                        rusqlite::types::Type::Text,
                    )
                })?;
                Ok(PendingLaunch {
                    request_id: row.get(0)?,
                    parent_id: row.get(1)?,
                    project_id: row.get(2)?,
                    tool,
                    title: row.get(4)?,
                    selected_cwd: PathBuf::from(row.get::<_, String>(5)?),
                    handoff_text: row.get(6)?,
                    handoff_path: row.get::<_, Option<String>>(7)?.map(Into::into),
                    state: row.get(8)?,
                    started_at_ms: row.get(9)?,
                    observed_pane: row.get(10)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn resolve_pending(&mut self, request_id: &str, record: &SessionRecord) -> Result<()> {
        let transaction = self.connection.transaction()?;
        let (project_id, parent_id): (String, Option<String>) = transaction.query_row(
            "SELECT project_id, parent_id FROM pending_launch WHERE request_id=?1",
            [request_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if project_id != record.project_id || parent_id != record.parent_id {
            bail!("pending launch and session relationship differ");
        }
        validate_parent(
            &transaction,
            &record.id,
            &record.project_id,
            record.parent_id.as_deref(),
        )?;
        transaction.execute(
            "INSERT INTO session (id, project_id, parent_id, title_override, selected_cwd,
             handoff_text, handoff_path, work_state, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                record.id,
                record.project_id,
                record.parent_id,
                record.title_override,
                path_text(record.selected_cwd.as_deref()),
                record.handoff_text,
                path_text(record.handoff_path.as_deref()),
                record.work_state,
                record.created_at_ms,
                record.updated_at_ms
            ],
        )?;
        transaction.execute(
            "DELETE FROM pending_launch WHERE request_id=?1",
            [request_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Merge scanned native metadata with sparse records. Every scan uses the same stable project IDs.
    pub fn merge(&mut self, native: IndexSnapshot) -> Result<IndexSnapshot> {
        let saved = self.sessions()?;
        let saved_by_id: HashMap<_, _> =
            saved.iter().map(|item| (item.id.as_str(), item)).collect();
        let mut sessions = Vec::new();
        let mut seen = HashSet::new();
        for mut session in native.sessions {
            if !seen.insert(session.id.clone()) {
                continue;
            }
            if let Some(record) = saved_by_id.get(session.id.as_str()) {
                session.project_id = record.project_id.clone();
                session.parent_id = record.parent_id.clone();
                session.selected_cwd = record.selected_cwd.clone();
                session.work_state = record.work_state.clone();
            } else {
                let root = normalize_directory(&session.directory);
                let name = root
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("/");
                let project = self.create_project(name, &root, ProjectOrigin::Auto)?;
                session.project_id = project.id;
            }
            sessions.push(session);
        }
        for record in saved {
            if seen.contains(&record.id) {
                continue;
            }
            let (agent, native_id) = parse_key(&record.id)?;
            let project = self
                .projects()?
                .into_iter()
                .find(|project| project.id == record.project_id)
                .with_context(|| format!("project {} missing", record.project_id))?;
            let mut session = AiSession::new(
                agent,
                native_id,
                record
                    .title_override
                    .clone()
                    .unwrap_or_else(|| format!("Unavailable session {}", native_id)),
                record
                    .selected_cwd
                    .clone()
                    .unwrap_or(project.root_directory),
                Some(record.created_at_ms),
                Some(record.updated_at_ms),
            );
            session.project_id = record.project_id;
            session.parent_id = record.parent_id;
            session.selected_cwd = record.selected_cwd;
            session.work_state = record.work_state;
            session.native_available = false;
            sessions.push(session);
        }
        Ok(build_snapshot_with_projects(
            sessions,
            self.projects()?,
            native.warnings,
        ))
    }
}

fn validate_parent(
    connection: &Connection,
    id: &str,
    project_id: &str,
    parent_id: Option<&str>,
) -> Result<()> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM project WHERE id=?1)",
        [project_id],
        |row| row.get(0),
    )?;
    if !exists {
        bail!("project {project_id} does not exist");
    }
    let Some(parent_id) = parent_id else {
        return Ok(());
    };
    if id == parent_id {
        bail!("a session cannot be its own parent");
    }
    let mut cursor = Some(parent_id.to_owned());
    let mut visited = HashSet::new();
    while let Some(current) = cursor {
        if current == id || !visited.insert(current.clone()) {
            bail!("parent relationship would form a cycle");
        }
        let row: Option<(String, Option<String>)> = connection
            .query_row(
                "SELECT project_id, parent_id FROM session WHERE id=?1",
                [&current],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((parent_project, next)) = row else {
            bail!("parent session {current} is not saved");
        };
        if parent_project != project_id {
            bail!("parent session belongs to another project");
        }
        cursor = next;
    }
    Ok(())
}

fn parse_key(id: &str) -> Result<(AgentKind, &str)> {
    let (tool, native_id) = id
        .split_once(':')
        .context("session key needs tool:native-id")?;
    let agent = AgentKind::from_command_name(tool).context("unknown session tool")?;
    if native_id.is_empty() {
        bail!("native session ID cannot be empty");
    }
    Ok((agent, native_id))
}

fn path_text(path: Option<&Path>) -> Option<String> {
    path.map(|path| path.to_string_lossy().into_owned())
}

fn project_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    let origin: String = row.get(3)?;
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        root_directory: PathBuf::from(row.get::<_, String>(2)?),
        origin: if origin == "manual" {
            ProjectOrigin::Manual
        } else {
            ProjectOrigin::Auto
        },
    })
}

fn session_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRecord> {
    Ok(SessionRecord {
        id: row.get(0)?,
        project_id: row.get(1)?,
        parent_id: row.get(2)?,
        title_override: row.get(3)?,
        selected_cwd: row.get::<_, Option<String>>(4)?.map(Into::into),
        handoff_text: row.get(5)?,
        handoff_path: row.get::<_, Option<String>>(6)?.map(Into::into),
        work_state: row.get(7)?,
        created_at_ms: row.get(8)?,
        updated_at_ms: row.get(9)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    use zellij_ai_session_core::{AgentKind, build_snapshot};

    fn database_path() -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("workbench-{}-{stamp}.db", std::process::id()))
    }

    fn record(id: &str, project_id: &str) -> SessionRecord {
        SessionRecord {
            id: id.into(),
            project_id: project_id.into(),
            parent_id: None,
            title_override: None,
            selected_cwd: None,
            handoff_text: None,
            handoff_path: None,
            work_state: None,
            created_at_ms: 1,
            updated_at_ms: 2,
        }
    }

    #[test]
    fn saved_relationships_survive_reopen_and_missing_native_history() {
        let path = database_path();
        let mut store = WorkbenchStore::open(&path).unwrap();
        let parent = AiSession::new(
            AgentKind::Codex,
            "parent",
            "Native title".into(),
            "/tmp/workbench-test".into(),
            Some(1),
            Some(2),
        );
        let child = AiSession::new(
            AgentKind::OpenCode,
            "child",
            "Child".into(),
            "/tmp/workbench-test".into(),
            Some(3),
            Some(4),
        );
        let first = store
            .merge(build_snapshot(vec![parent.clone(), child.clone()], vec![]))
            .unwrap();
        assert_eq!(first.projects.len(), 1);
        let project_id = first.projects[0].project.id.clone();
        let mut parent_record = record(&parent.id, &project_id);
        parent_record.title_override = Some("My plan".into());
        store.save_session(&parent_record).unwrap();
        let mut child_record = record(&child.id, &project_id);
        child_record.parent_id = Some(parent.id.clone());
        store.save_session(&child_record).unwrap();
        drop(store);

        let mut reopened = WorkbenchStore::open(&path).unwrap();
        let merged = reopened
            .merge(build_snapshot(vec![parent], vec![]))
            .unwrap();
        assert_eq!(merged.projects[0].project.id, project_id);
        assert_eq!(merged.sessions.len(), 2);
        let saved_parent = merged
            .sessions
            .iter()
            .find(|item| item.id == "codex:parent")
            .unwrap();
        assert_eq!(saved_parent.title, "Native title");
        let missing_child = merged
            .sessions
            .iter()
            .find(|item| item.id == "opencode:child")
            .unwrap();
        assert_eq!(missing_child.parent_id.as_deref(), Some("codex:parent"));
        assert!(!missing_child.native_available);
        drop(reopened);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_cross_project_and_cyclic_parents() {
        let path = database_path();
        let mut store = WorkbenchStore::open(&path).unwrap();
        let a = store
            .create_project("a", Path::new("/tmp/a"), ProjectOrigin::Manual)
            .unwrap();
        let b = store
            .create_project("b", Path::new("/tmp/b"), ProjectOrigin::Manual)
            .unwrap();
        store.save_session(&record("codex:one", &a.id)).unwrap();
        store.save_session(&record("codex:two", &a.id)).unwrap();
        store.save_session(&record("codex:other", &b.id)).unwrap();
        assert!(store.set_parent("codex:one", Some("codex:other")).is_err());
        store.set_parent("codex:two", Some("codex:one")).unwrap();
        assert!(store.set_parent("codex:one", Some("codex:two")).is_err());
        assert!(store.set_parent("codex:one", Some("codex:one")).is_err());
        assert_eq!(store.session("codex:one").unwrap().unwrap().parent_id, None);
        drop(store);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn pending_launch_survives_reopen() {
        let path = database_path();
        let mut store = WorkbenchStore::open(&path).unwrap();
        let project = store
            .create_project("a", Path::new("/tmp/a"), ProjectOrigin::Manual)
            .unwrap();
        let pending = PendingLaunch {
            request_id: "request-1".into(),
            parent_id: None,
            project_id: project.id,
            tool: AgentKind::Codex,
            title: "New".into(),
            selected_cwd: "/tmp/a".into(),
            handoff_text: Some("Do work".into()),
            handoff_path: None,
            state: "prepared".into(),
            started_at_ms: None,
            observed_pane: None,
        };
        store.save_pending(&pending).unwrap();
        drop(store);
        let reopened = WorkbenchStore::open(&path).unwrap();
        assert_eq!(reopened.pending().unwrap(), vec![pending]);
        drop(reopened);
        std::fs::remove_file(path).unwrap();
    }
}
