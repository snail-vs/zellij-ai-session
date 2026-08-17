mod adapters;
mod agents;
mod claude;
mod codewhale;
mod codex;
mod goose;
mod opencode;
mod pi;
mod qwen;
mod reasonix;

use std::path::PathBuf;

use anyhow::Result;
use zellij_ai_session_core::{
    AiSession, CommandSpec, IndexSnapshot, RuntimeRef, SessionStatus, build_snapshot,
};

pub use adapters::{AdapterContext, AgentAdapter};

#[derive(Debug, Clone, Default)]
pub struct IndexerConfig {
    pub codex_home: Option<PathBuf>,
    pub opencode_db: Option<PathBuf>,
}

pub struct Indexer {
    adapters: Vec<Box<dyn AgentAdapter>>,
}

impl Indexer {
    pub fn from_config(config: IndexerConfig) -> Self {
        let mut adapters: Vec<Box<dyn AgentAdapter>> = Vec::new();
        for (_, build) in agents::ADAPTER_BUILDERS {
            if let Some(adapter) = build(&config) {
                adapters.push(adapter);
            }
        }
        Self { adapters }
    }

    pub fn scan(&self) -> IndexSnapshot {
        let mut sessions = Vec::new();
        let mut warnings = Vec::new();

        for adapter in &self.adapters {
            match adapter.list_sessions() {
                Ok(mut found) => sessions.append(&mut found),
                Err(error) => warnings.push(format!("{}: {error}", adapter.name())),
            }
        }

        build_snapshot(sessions, warnings)
    }

    pub fn resume_command(&self, session: &AiSession) -> Result<CommandSpec> {
        self.adapters
            .iter()
            .find(|adapter| adapter.agent() == session.agent)
            .ok_or_else(|| anyhow::anyhow!("no adapter registered for {}", session.agent))?
            .resume_command(session)
    }
}

pub fn apply_runtime(sessions: &mut [AiSession], runtimes: &[RuntimeRef]) {
    for session in sessions {
        let candidates: Vec<&RuntimeRef> = runtimes
            .iter()
            .filter(|runtime| {
                runtime.cwd.as_deref() == Some(session.directory.as_path())
                    && runtime
                        .command
                        .as_deref()
                        .is_some_and(|command| command.contains(session.agent.command_name()))
            })
            .collect();
        let exact = candidates.iter().find(|runtime| {
            runtime
                .command
                .as_deref()
                .is_some_and(|command| command.contains(&session.agent_session_id))
        });
        let runtime = exact
            .or_else(|| candidates.first().filter(|_| candidates.len() == 1))
            .copied();
        let Some(runtime) = runtime else {
            continue;
        };
        session.status = SessionStatus::Running;
        session.runtime = Some(runtime.clone());
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use zellij_ai_session_core::{AgentKind, RuntimeConfidence};

    #[test]
    fn runtime_matching_marks_session_running() {
        let project =
            zellij_ai_session_core::project_for_directory(std::path::Path::new("/tmp/app"));
        let mut sessions = vec![AiSession {
            id: "codex:one".into(),
            agent: AgentKind::Codex,
            title: "one".into(),
            project_id: project.id,
            directory: PathBuf::from("/tmp/app"),
            created_at_ms: None,
            updated_at_ms: None,
            agent_session_id: "one".into(),
            status: SessionStatus::Historical,
            runtime: None,
        }];
        apply_runtime(
            &mut sessions,
            &[RuntimeRef {
                zellij_session: Some("dev".into()),
                tab_id: Some(1),
                pane_id: Some(2),
                cwd: Some(PathBuf::from("/tmp/app")),
                command: Some("codex resume one".into()),
                confidence: RuntimeConfidence::Heuristic,
            }],
        );
        assert_eq!(sessions[0].status, SessionStatus::Running);
    }
}
