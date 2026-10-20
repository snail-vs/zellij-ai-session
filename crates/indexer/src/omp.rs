use std::path::PathBuf;

use anyhow::Result;
use zellij_ai_session_core::{AgentKind, AiSession, CommandSpec, SessionPreview};

use crate::adapters::{AgentAdapter, preview_result};
use crate::pi_format;

/// Adapter for OMP (oh-my-pi), the fork of Pi that stores its sessions in
/// `~/.omp/agent/sessions`.
///
/// OMP writes the same JSONL transcript as Pi, but places its auto-generated
/// `title` entry on the first line and moves the `session` entry to the second
/// one, and it addresses sessions by uuid rather than by file path.
pub struct OmpAdapter {
    sessions_root: PathBuf,
}

impl OmpAdapter {
    pub fn new(sessions_root: PathBuf) -> Self {
        Self { sessions_root }
    }

    /// Collect the sessions and the problems met on the way.
    ///
    /// OMP garbage collects old sessions on its own, so a file can disappear
    /// while the workbench is scanning. A single unreadable file must not hide
    /// every other session of the tool.
    fn scan(&self) -> Result<(Vec<AiSession>, Vec<String>)> {
        let mut sessions = Vec::new();
        let mut warnings = Vec::new();
        for path in pi_format::session_files(&self.sessions_root) {
            let header = match pi_format::read_header(&path) {
                Ok(Some(header)) => header,
                Ok(None) => continue,
                Err(error) => {
                    warnings.push(format!("{}: {error}", path.display()));
                    continue;
                }
            };
            if header.cwd.as_os_str().is_empty() {
                continue;
            }
            let title = header
                .title
                .unwrap_or_else(|| format!("OMP session {}", &header.id[..header.id.len().min(8)]));
            sessions.push(AiSession::new(
                AgentKind::Omp,
                &header.id,
                title,
                header.cwd,
                header.created_at_ms,
                header.updated_at_ms,
            ));
        }
        Ok((sessions, warnings))
    }
}

impl AgentAdapter for OmpAdapter {
    fn name(&self) -> &'static str {
        "OMP"
    }

    fn agent(&self) -> AgentKind {
        AgentKind::Omp
    }

    fn list_sessions(&self) -> Result<Vec<AiSession>> {
        Ok(self.scan()?.0)
    }

    fn list_sessions_with_warnings(&self) -> Result<(Vec<AiSession>, Vec<String>)> {
        self.scan()
    }

    fn resume_command(&self, session: &AiSession) -> Result<CommandSpec> {
        Ok(CommandSpec::new("omp", session.directory.clone())
            .with_args(["--resume", session.agent_session_id.as_str()]))
    }

    fn preview(&self, session_id: &str) -> Result<SessionPreview> {
        let Some(path) = pi_format::find_session_file(&self.sessions_root, session_id) else {
            return Ok(preview_result(session_id, false, Vec::new()));
        };
        let messages = pi_format::read_messages(&path)?;
        Ok(preview_result(session_id, true, messages))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use zellij_ai_session_core::SessionStatus;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("omp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("--tmp-demo--")).unwrap();
        root
    }

    fn write_session(root: &Path, id: &str, title: &str, cwd: &str) {
        let contents = format!(
            "{{\"type\":\"title\",\"v\":1,\"title\":\"{title}\",\"source\":\"auto\"}}\n\
             {{\"type\":\"session\",\"version\":3,\"id\":\"{id}\",\"timestamp\":\"2026-01-02T03:04:05.000Z\",\"cwd\":\"{cwd}\"}}\n\
             {{\"type\":\"message\",\"timestamp\":\"2026-01-02T03:05:00.000Z\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"hello omp\"}}]}}}}\n"
        );
        std::fs::write(
            root.join("--tmp-demo--")
                .join(format!("2026-01-02T03-04-05-000Z_{id}.jsonl")),
            contents,
        )
        .unwrap();
    }

    #[test]
    fn lists_sessions_with_their_omp_title() {
        let root = temp_root("list");
        write_session(&root, "omp-id", "OMP title", "/tmp/demo");
        let adapter = OmpAdapter::new(root.clone());
        let sessions = adapter.list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].agent, AgentKind::Omp);
        assert_eq!(sessions[0].agent_session_id, "omp-id");
        assert_eq!(sessions[0].title, "OMP title");
        assert_eq!(sessions[0].directory, PathBuf::from("/tmp/demo"));
        assert_eq!(sessions[0].status, SessionStatus::Historical);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn previews_a_session_by_its_uuid() {
        let root = temp_root("preview");
        write_session(&root, "omp-id", "OMP title", "/tmp/demo");
        let adapter = OmpAdapter::new(root.clone());
        let preview = adapter.preview("omp-id").unwrap();
        assert_eq!(preview.messages.len(), 1);
        assert_eq!(preview.messages[0].text, "hello omp");
        assert!(preview.note.is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reports_a_missing_session() {
        let root = temp_root("missing");
        let preview = OmpAdapter::new(root.clone()).preview("nope").unwrap();
        assert!(preview.messages.is_empty());
        assert!(preview.note.is_some());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_unreadable_file_does_not_hide_the_other_sessions() {
        let root = temp_root("unreadable");
        write_session(&root, "omp-id", "OMP title", "/tmp/demo");
        // A directory named like a session file cannot be opened.
        std::fs::create_dir_all(
            root.join("--tmp-demo--")
                .join("2026-01-02T03-04-05-000Z_broken.jsonl"),
        )
        .unwrap();
        let (sessions, warnings) = OmpAdapter::new(root.clone())
            .list_sessions_with_warnings()
            .unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].agent_session_id, "omp-id");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("broken.jsonl"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resumes_with_the_session_uuid() {
        let root = temp_root("resume");
        write_session(&root, "omp-id", "OMP title", "/tmp/demo");
        let session = OmpAdapter::new(root.clone())
            .list_sessions()
            .unwrap()
            .remove(0);
        let command = OmpAdapter::new(root.clone())
            .resume_command(&session)
            .unwrap();
        assert_eq!(command.program, "omp");
        assert_eq!(command.args, vec!["--resume", "omp-id"]);
        assert_eq!(command.cwd, PathBuf::from("/tmp/demo"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
