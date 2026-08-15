use anyhow::Result;
use zellij_ai_session_core::{AgentKind, AiSession, CommandSpec};

pub trait AgentAdapter: Send + Sync {
    fn name(&self) -> &'static str;
    fn agent(&self) -> AgentKind;
    fn list_sessions(&self) -> Result<Vec<AiSession>>;
    fn resume_command(&self, session: &AiSession) -> Result<CommandSpec>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AdapterContext;
