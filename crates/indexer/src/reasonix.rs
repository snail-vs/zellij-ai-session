use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use zellij_ai_session_core::{AgentKind, AiSession, CommandSpec, SessionPreview};

use crate::adapters::{AgentAdapter, preview_message, preview_result};

#[derive(Debug, Deserialize, Default)]
struct ReasonixMeta {
    #[serde(rename = "WorkspaceRoot", default)]
    workspace_root: Option<String>,
    #[serde(rename = "TopicTitle", default)]
    topic_title: Option<String>,
    #[serde(rename = "Preview", default)]
    preview: Option<String>,
    #[serde(rename = "Turns", default)]
    turns: Option<u64>,
    #[serde(rename = "CreatedAt", default)]
    created_at: Option<String>,
    #[serde(rename = "UpdatedAt", default)]
    updated_at: Option<String>,
}

pub struct ReasonixAdapter {
    sessions_dir: PathBuf,
}

impl ReasonixAdapter {
    pub fn new(sessions_dir: PathBuf) -> Self {
        Self { sessions_dir }
    }

    fn read_meta(path: &Path) -> Option<ReasonixMeta> {
        for suffix in ["meta.json", "meta"] {
            let candidate = path.with_extension(suffix);
            if let Ok(file) = File::open(&candidate) {
                if let Ok(meta) = serde_json::from_reader::<_, ReasonixMeta>(BufReader::new(file)) {
                    return Some(meta);
                }
            }
        }
        None
    }

    fn parse_file(&self, path: &Path) -> Result<Option<AiSession>> {
        let meta = Self::read_meta(path);
        // Reasonix skips sessions with zero turns (never had user input).
        if meta.as_ref().and_then(|meta| meta.turns).unwrap_or(1) == 0 {
            return Ok(None);
        }

        let directory: PathBuf = match meta.as_ref().and_then(|meta| meta.workspace_root.clone()) {
            Some(root) if !root.is_empty() => root.into(),
            _ => self.cwd_from_jsonl(path)?,
        };
        let title = meta
            .as_ref()
            .and_then(|meta| meta.topic_title.clone().filter(|title| !title.is_empty()))
            .or_else(|| {
                meta.as_ref()
                    .and_then(|meta| meta.preview.clone().filter(|p| !p.is_empty()))
            })
            .or_else(|| self.preview_from_jsonl(path).ok().flatten())
            .unwrap_or_else(|| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("reasonix")
                    .to_string()
            });

        let created_at_ms = meta
            .as_ref()
            .and_then(|meta| meta.created_at.as_deref())
            .and_then(crate::adapters::iso_to_ms)
            .or_else(|| crate::adapters::file_mtime_ms(path));
        let updated_at_ms = meta
            .as_ref()
            .and_then(|meta| meta.updated_at.as_deref())
            .and_then(crate::adapters::iso_to_ms)
            .or(created_at_ms);

        let agent_session_id = path.to_string_lossy().into_owned();
        Ok(Some(AiSession::new(
            AgentKind::Reasonix,
            &agent_session_id,
            title,
            directory,
            created_at_ms,
            updated_at_ms,
        )))
    }

    /// Best-effort: scan the first lines of the transcript for a workspace root
    /// field, used only when the `.meta.json` sidecar is missing.
    fn cwd_from_jsonl(&self, path: &Path) -> Result<PathBuf> {
        let file = File::open(path)
            .with_context(|| format!("open Reasonix session {}", path.display()))?;
        let reader = BufReader::new(file);
        for (index, line) in reader.lines().enumerate() {
            let line = match line {
                Ok(line) => line,
                Err(_) => break,
            };
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                for key in ["workspaceRoot", "cwd"] {
                    if let Some(root) = value.get(key).and_then(|value| value.as_str()) {
                        if !root.is_empty() {
                            return Ok(root.into());
                        }
                    }
                }
            }
            if index >= 50 {
                break;
            }
        }
        Ok(PathBuf::new())
    }

    fn preview_from_jsonl(&self, path: &Path) -> Result<Option<String>> {
        let file = File::open(path)
            .with_context(|| format!("open Reasonix session {}", path.display()))?;
        let reader = BufReader::new(file);
        for (index, line) in reader.lines().enumerate() {
            let line = match line {
                Ok(line) => line,
                Err(_) => break,
            };
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                if value.get("role").and_then(|value| value.as_str()) == Some("user") {
                    if let Some(text) = value.get("content").and_then(crate::adapters::first_text) {
                        return Ok(Some(text.chars().take(200).collect()));
                    }
                }
            }
            if index >= 200 {
                break;
            }
        }
        Ok(None)
    }
}

impl AgentAdapter for ReasonixAdapter {
    fn name(&self) -> &'static str {
        "Reasonix"
    }

    fn agent(&self) -> AgentKind {
        AgentKind::Reasonix
    }

    fn list_sessions(&self) -> Result<Vec<AiSession>> {
        let mut result = Vec::new();
        if !self.sessions_dir.exists() {
            return Ok(result);
        }
        let mut scan = |path: &Path| {
            if let Some(session) = self.parse_file(path)? {
                result.push(session);
            }
            Ok(())
        };
        for root in [
            self.sessions_dir.join("sessions"),
            self.sessions_dir.join("projects"),
        ] {
            if root.is_dir() {
                self.visit_sessions(&root, &mut scan)?;
            }
        }
        if !self.sessions_dir.join("sessions").is_dir()
            && !self.sessions_dir.join("projects").is_dir()
        {
            self.visit_sessions(&self.sessions_dir, &mut scan)?;
        }
        Ok(result)
    }

    fn resume_command(&self, session: &AiSession) -> Result<CommandSpec> {
        Ok(CommandSpec::new("reasonix", session.directory.clone())
            .with_args(["--resume", session.agent_session_id.as_str()]))
    }

    fn preview(&self, session_id: &str) -> Result<SessionPreview> {
        let path = Path::new(session_id);
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl")
            || !path.canonicalize().ok().is_some_and(|path| {
                self.sessions_dir
                    .canonicalize()
                    .ok()
                    .is_some_and(|root| path.starts_with(root))
            })
        {
            return Ok(preview_result(session_id, false, Vec::new()));
        }
        let mut messages = Vec::new();
        for line in BufReader::new(File::open(path)?).lines() {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&line?) else {
                continue;
            };
            let Some(role) = value.get("role").and_then(|v| v.as_str()) else {
                continue;
            };
            if let Some(content) = value.get("content") {
                if let Some(message) = preview_message(role, content) {
                    messages.push(message);
                }
            }
        }
        Ok(preview_result(session_id, true, messages))
    }
}

impl ReasonixAdapter {
    fn visit_sessions(
        &self,
        root: &Path,
        callback: &mut impl FnMut(&Path) -> Result<()>,
    ) -> Result<()> {
        for entry in std::fs::read_dir(root).with_context(|| format!("read {}", root.display()))? {
            let path = entry?.path();
            if path.is_dir() {
                if path.file_name().and_then(|name| name.to_str()) == Some("sessions") {
                    self.visit_sessions(&path, callback)?;
                } else if root == self.sessions_dir || root == self.sessions_dir.join("projects") {
                    self.visit_sessions(&path, callback)?;
                }
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl")
                && !path.to_string_lossy().ends_with(".events.jsonl")
            {
                callback(&path)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "zais-rx-{}-{}",
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
    fn reads_meta_sidecar_for_title_and_cwd() {
        let root = temp_root("meta");
        let session = root.join("code-project.jsonl");
        std::fs::write(
            &session,
            r#"{"role":"user","content":"first message"}
{"role":"assistant","content":"reply"}
"#,
        )
        .unwrap();
        std::fs::write(
            session.with_extension("meta.json"),
            r#"{"WorkspaceRoot":"/home/user/project","TopicTitle":"Build parser","Preview":"first message","Turns":5,"CreatedAt":"2026-02-01T09:00:00.000Z","UpdatedAt":"2026-02-01T09:30:00.000Z"}"#,
        )
        .unwrap();

        let adapter = ReasonixAdapter::new(root.clone());
        let sessions = adapter.list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.agent, AgentKind::Reasonix);
        assert_eq!(session.title, "Build parser");
        assert_eq!(session.directory, PathBuf::from("/home/user/project"));
        assert_eq!(session.created_at_ms, Some(1_769_936_400_000));
        assert_eq!(session.updated_at_ms, Some(1_769_938_200_000));
        assert!(session.agent_session_id.ends_with("code-project.jsonl"));

        let command = adapter.resume_command(session).unwrap();
        assert_eq!(command.program, "reasonix");
        assert_eq!(
            command.args,
            vec!["--resume", session.agent_session_id.as_str()]
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn skips_zero_turn_sessions() {
        let root = temp_root("empty");
        let session = root.join("code-empty.jsonl");
        std::fs::write(&session, r#"{"role":"user","content":"hi"}"#).unwrap();
        std::fs::write(
            session.with_extension("meta.json"),
            r#"{"WorkspaceRoot":"/p","TopicTitle":"x","Turns":0}"#,
        )
        .unwrap();

        let sessions = ReasonixAdapter::new(root.clone()).list_sessions().unwrap();
        assert!(sessions.is_empty());

        std::fs::remove_dir_all(&root).ok();
    }
}
