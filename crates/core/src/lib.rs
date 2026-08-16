use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const SNAPSHOT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    Codex,
    OpenCode,
}

impl AgentKind {
    pub fn command_name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
        }
    }
}

impl std::fmt::Display for AgentKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.command_name())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub root_directory: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Historical,
    Running,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeRef {
    pub zellij_session: Option<String>,
    pub tab_id: Option<u32>,
    pub pane_id: Option<u32>,
    pub cwd: Option<PathBuf>,
    pub command: Option<String>,
    pub confidence: RuntimeConfidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeConfidence {
    Exact,
    Heuristic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiSession {
    pub id: String,
    pub agent: AgentKind,
    pub title: String,
    pub project_id: String,
    pub directory: PathBuf,
    pub created_at_ms: Option<i64>,
    pub updated_at_ms: Option<i64>,
    pub agent_session_id: String,
    pub status: SessionStatus,
    pub runtime: Option<RuntimeRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub project: Project,
    pub session_count: usize,
    pub running_count: usize,
    pub latest_updated_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexSnapshot {
    pub version: u32,
    pub generated_at_ms: i64,
    pub projects: Vec<ProjectSummary>,
    pub sessions: Vec<AiSession>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
}

impl CommandSpec {
    pub fn new(program: impl Into<String>, cwd: PathBuf) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd,
        }
    }

    pub fn with_args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }
}

pub fn normalize_directory(path: &Path) -> PathBuf {
    let expanded = if path == Path::new("~") {
        home_directory().unwrap_or_else(|| path.to_path_buf())
    } else if let Ok(stripped) = path.strip_prefix("~/") {
        home_directory()
            .map(|home| home.join(stripped))
            .unwrap_or_else(|| path.to_path_buf())
    } else {
        path.to_path_buf()
    };

    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir().unwrap_or_default().join(expanded)
    };

    let candidate = std::fs::canonicalize(&absolute).unwrap_or(absolute);
    let mut normalized = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

pub fn project_for_directory(directory: &Path) -> Project {
    let root_directory = normalize_directory(directory);
    let name = root_directory
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("/")
        .to_string();
    let id = root_directory.to_string_lossy().into_owned();
    Project {
        id,
        name,
        root_directory,
    }
}

pub fn search_key(value: &str) -> String {
    value.chars().flat_map(char::to_lowercase).collect()
}

pub fn build_snapshot(mut sessions: Vec<AiSession>, warnings: Vec<String>) -> IndexSnapshot {
    sessions.sort_by(|left, right| session_sort_key(left).cmp(&session_sort_key(right)));

    let mut projects: Vec<ProjectSummary> = Vec::new();
    for session in &sessions {
        if let Some(summary) = projects
            .iter_mut()
            .find(|summary| summary.project.id == session.project_id)
        {
            summary.session_count += 1;
            if session.status == SessionStatus::Running {
                summary.running_count += 1;
            }
            summary.latest_updated_at_ms =
                max_timestamp(summary.latest_updated_at_ms, session.updated_at_ms);
            continue;
        }

        let project = project_for_directory(&session.directory);
        projects.push(ProjectSummary {
            project,
            session_count: 1,
            running_count: usize::from(session.status == SessionStatus::Running),
            latest_updated_at_ms: session.updated_at_ms,
        });
    }

    projects.sort_by(|left, right| {
        right
            .running_count
            .cmp(&left.running_count)
            .then_with(|| right.latest_updated_at_ms.cmp(&left.latest_updated_at_ms))
            .then_with(|| {
                left.project
                    .name
                    .to_lowercase()
                    .cmp(&right.project.name.to_lowercase())
            })
    });

    IndexSnapshot {
        version: SNAPSHOT_VERSION,
        generated_at_ms: now_ms(),
        projects,
        sessions,
        warnings,
    }
}

fn session_sort_key(session: &AiSession) -> (bool, i64, &str) {
    (
        session.status == SessionStatus::Running,
        session.updated_at_ms.unwrap_or_default(),
        &session.title,
    )
}

fn max_timestamp(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn home_directory() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str, directory: &str, status: SessionStatus, updated_at_ms: i64) -> AiSession {
        let project = project_for_directory(Path::new(directory));
        AiSession {
            id: id.to_string(),
            agent: AgentKind::Codex,
            title: id.to_string(),
            project_id: project.id,
            directory: PathBuf::from(directory),
            created_at_ms: Some(updated_at_ms),
            updated_at_ms: Some(updated_at_ms),
            agent_session_id: id.to_string(),
            status,
            runtime: None,
        }
    }

    #[test]
    fn groups_sessions_by_normalized_directory() {
        let snapshot = build_snapshot(
            vec![
                session(
                    "old",
                    "/tmp/project/../project",
                    SessionStatus::Historical,
                    1,
                ),
                session("new", "/tmp/project", SessionStatus::Running, 2),
            ],
            Vec::new(),
        );

        assert_eq!(snapshot.projects.len(), 1);
        assert_eq!(snapshot.projects[0].session_count, 2);
        assert_eq!(snapshot.projects[0].running_count, 1);
    }

    #[test]
    fn running_projects_sort_first() {
        let snapshot = build_snapshot(
            vec![
                session("historical", "/tmp/a", SessionStatus::Historical, 100),
                session("running", "/tmp/b", SessionStatus::Running, 1),
            ],
            Vec::new(),
        );

        assert_eq!(snapshot.projects[0].project.name, "b");
    }

    #[test]
    fn search_key_supports_unicode_text() {
        assert!(search_key("修复中文搜索").contains(&search_key("中文搜索")));
    }
}
