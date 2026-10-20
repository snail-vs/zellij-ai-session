use std::path::PathBuf;

use zellij_ai_session_core::AgentKind;

use crate::adapters::AgentAdapter;
use crate::{
    IndexerConfig, claude::ClaudeAdapter, codewhale::CodewhaleAdapter, codex::CodexAdapter,
    cursor::CursorAdapter, goose::GooseAdapter, omp::OmpAdapter, opencode::OpenCodeAdapter,
    pi::PiAdapter, qwen::QwenAdapter, reasonix::ReasonixAdapter,
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
    (AgentKind::Omp, build_omp),
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

/// OMP stores its sessions in `<agent dir>/sessions`. It resolves that
/// directory the same way the tool itself does: `PI_CODING_AGENT_DIR` wins,
/// otherwise the default `~/.omp/agent` is used. The `~/.pi/agent` fallback of
/// the tool is left out, because those sessions are already indexed as Pi.
fn build_omp(config: &IndexerConfig) -> Option<Box<dyn AgentAdapter>> {
    let agent_dir = config
        .omp_dir
        .clone()
        .or_else(|| {
            std::env::var_os("PI_CODING_AGENT_DIR")
                .map(PathBuf::from)
                .filter(|dir| !dir.as_os_str().is_empty())
        })
        .or_else(|| home().map(|base| base.join(".omp/agent")))?;
    let sessions_root = agent_dir.join("sessions");
    // The same directory is already indexed as Pi, so do not list it twice.
    if home().is_some_and(|base| sessions_root == base.join(".pi/agent/sessions")) {
        return None;
    }
    Some(Box::new(OmpAdapter::new(sessions_root)))
}

fn build_reasonix(_config: &IndexerConfig) -> Option<Box<dyn AgentAdapter>> {
    let base = std::env::var_os("REASONIX_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("REASONIX_HOME").map(PathBuf::from))
        .or_else(|| home().map(|base| base.join(".reasonix")))?;
    Some(Box::new(ReasonixAdapter::new(base)))
}

fn build_codewhale(_config: &IndexerConfig) -> Option<Box<dyn AgentAdapter>> {
    let base = std::env::var_os("CODEWHALE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            home().map(|base| {
                let current = base.join(".codewhale");
                if current.join("sessions").is_dir() {
                    current
                } else {
                    base.join(".deepseek")
                }
            })
        })?;
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
