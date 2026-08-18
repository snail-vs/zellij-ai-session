use std::path::PathBuf;

use zellij_ai_session_core::AgentKind;

use crate::adapters::AgentAdapter;
use crate::{
    IndexerConfig, claude::ClaudeAdapter, codewhale::CodewhaleAdapter, codex::CodexAdapter,
    cursor::CursorAdapter, goose::GooseAdapter, opencode::OpenCodeAdapter, pi::PiAdapter,
    qwen::QwenAdapter, reasonix::ReasonixAdapter,
};

/// Builds an adapter from config, returning `None` when the agent's session
/// store is not present on this machine.
pub type AdapterBuilder = fn(&IndexerConfig) -> Option<Box<dyn AgentAdapter>>;

/// Registry mapping each `AgentKind` to the function that constructs its
/// adapter. Adding a new agent means adding one row here plus its
/// `AgentAdapter` implementation.
pub const ADAPTER_BUILDERS: &[(AgentKind, AdapterBuilder)] = &[
    (AgentKind::Codex, build_codex),
    (AgentKind::OpenCode, build_opencode),
    (AgentKind::Cursor, build_cursor),
    (AgentKind::Pi, build_pi),
    (AgentKind::Reasonix, build_reasonix),
    (AgentKind::Codewhale, build_codewhale),
    (AgentKind::Claude, build_claude),
    (AgentKind::Qwen, build_qwen),
    (AgentKind::Goose, build_goose),
];

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Resolve `XDG_DATA_HOME/<xdg_sub>` falling back to `~/<home_sub>`.
fn xdg_or_home(xdg_sub: &str, home_sub: &str) -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(|base| PathBuf::from(base).join(xdg_sub))
        .or_else(|| home().map(|base| base.join(home_sub)))
}

fn build_codex(config: &IndexerConfig) -> Option<Box<dyn AgentAdapter>> {
    let base = config
        .codex_home
        .clone()
        .or_else(|| std::env::var_os("CODEX_HOME").map(PathBuf::from))
        .or_else(|| home().map(|base| base.join(".codex")))?;
    Some(Box::new(CodexAdapter::new(
        base.join("sessions"),
        base.join("session_index.jsonl"),
    )))
}

fn build_opencode(config: &IndexerConfig) -> Option<Box<dyn AgentAdapter>> {
    let database = config
        .opencode_db
        .clone()
        .or_else(|| xdg_or_home("opencode/opencode.db", ".local/share/opencode/opencode.db"))?;
    Some(Box::new(OpenCodeAdapter::new(database)))
}

fn build_cursor(config: &IndexerConfig) -> Option<Box<dyn AgentAdapter>> {
    let base = config
        .cursor_home
        .clone()
        .or_else(|| {
            std::env::var_os("CURSOR_CONFIG_DIR")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| home().map(|base| base.join(".cursor")))?;
    Some(Box::new(CursorAdapter::new(base.join("chats"))))
}

fn build_pi(_config: &IndexerConfig) -> Option<Box<dyn AgentAdapter>> {
    let sessions_root = home().map(|base| base.join(".pi/agent/sessions"))?;
    Some(Box::new(PiAdapter::new(sessions_root)))
}

fn build_reasonix(_config: &IndexerConfig) -> Option<Box<dyn AgentAdapter>> {
    let sessions_dir = std::env::var_os("REASONIX_STATE_HOME")
        .map(|base| PathBuf::from(base).join("sessions"))
        .or_else(|| {
            std::env::var_os("REASONIX_HOME").map(|base| PathBuf::from(base).join("sessions"))
        })
        .or_else(|| home().map(|base| base.join(".reasonix/sessions")))?;
    Some(Box::new(ReasonixAdapter::new(sessions_dir)))
}

fn build_codewhale(_config: &IndexerConfig) -> Option<Box<dyn AgentAdapter>> {
    let base = std::env::var_os("CODEWHALE_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|base| base.join(".codewhale")))
        .or_else(|| home().map(|base| base.join(".deepseek")))?;
    Some(Box::new(CodewhaleAdapter::new(base.join("sessions"))))
}

fn build_claude(_config: &IndexerConfig) -> Option<Box<dyn AgentAdapter>> {
    let base = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| home().map(|base| base.join(".claude")))?;
    Some(Box::new(ClaudeAdapter::new(base.join("projects"))))
}

fn build_qwen(_config: &IndexerConfig) -> Option<Box<dyn AgentAdapter>> {
    let base = std::env::var_os("QWEN_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| home().map(|base| base.join(".qwen")))?;
    Some(Box::new(QwenAdapter::new(base.join("projects"))))
}

fn build_goose(_config: &IndexerConfig) -> Option<Box<dyn AgentAdapter>> {
    let database = xdg_or_home(
        "goose/sessions/sessions.db",
        ".local/share/goose/sessions/sessions.db",
    )?;
    Some(Box::new(GooseAdapter::new(database)))
}
