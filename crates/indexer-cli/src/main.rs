use std::env;

use anyhow::Result;
use zellij_ai_session_core::{AgentKind, CommandSpec};
use zellij_ai_session_indexer::{Indexer, IndexerConfig};

fn main() -> Result<()> {
    let mut config = IndexerConfig::default();
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "scan".into());
    if command == "resume" {
        return resume_command(args.collect());
    }
    if command == "new" {
        return new_command(args.collect());
    }

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--codex-home" => config.codex_home = args.next().map(Into::into),
            "--opencode-db" => config.opencode_db = args.next().map(Into::into),
            "--help" | "-h" => {
                println!("zellij-ai-session-index [scan|resume|new] [options]");
                return Ok(());
            }
            unknown => anyhow::bail!("unknown argument: {unknown}"),
        }
    }

    let snapshot = Indexer::from_config(config).scan();
    println!("{}", serde_json::to_string_pretty(&snapshot)?);
    Ok(())
}

fn new_command(args: Vec<String>) -> Result<()> {
    let (agent, cwd) = parse_agent_and_cwd(args, "new")?;
    let command = match agent {
        AgentKind::Codex => CommandSpec::new("codex", cwd),
        AgentKind::OpenCode => CommandSpec::new("opencode", cwd),
    };
    println!("{}", serde_json::to_string(&command)?);
    Ok(())
}

fn parse_agent_and_cwd(
    args: Vec<String>,
    command_name: &str,
) -> Result<(AgentKind, std::path::PathBuf)> {
    let mut agent = None;
    let mut cwd = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--agent" => agent = args.next(),
            "--cwd" => cwd = args.next(),
            "--help" | "-h" => {
                println!(
                    "zellij-ai-session-index {command_name} --agent codex|opencode --cwd PATH"
                );
                return Err(anyhow::anyhow!("help requested"));
            }
            unknown => anyhow::bail!("unknown {command_name} argument: {unknown}"),
        }
    }
    let agent = match agent.as_deref() {
        Some("codex") => AgentKind::Codex,
        Some("opencode") => AgentKind::OpenCode,
        Some(other) => anyhow::bail!("unsupported agent: {other}"),
        None => anyhow::bail!("missing --agent"),
    };
    let cwd = cwd.ok_or_else(|| anyhow::anyhow!("missing --cwd"))?.into();
    Ok((agent, cwd))
}

fn resume_command(args: Vec<String>) -> Result<()> {
    let mut agent = None;
    let mut session_id = None;
    let mut cwd = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--agent" => agent = args.next(),
            "--session-id" => session_id = args.next(),
            "--cwd" => cwd = args.next(),
            "--help" | "-h" => {
                println!(
                    "zellij-ai-session-index resume --agent codex|opencode --session-id ID --cwd PATH"
                );
                return Ok(());
            }
            unknown => anyhow::bail!("unknown resume argument: {unknown}"),
        }
    }

    let agent = agent.ok_or_else(|| anyhow::anyhow!("missing --agent"))?;
    let session_id = session_id.ok_or_else(|| anyhow::anyhow!("missing --session-id"))?;
    let cwd = cwd.ok_or_else(|| anyhow::anyhow!("missing --cwd"))?.into();
    let agent = match agent.as_str() {
        "codex" => AgentKind::Codex,
        "opencode" => AgentKind::OpenCode,
        other => anyhow::bail!("unsupported agent: {other}"),
    };
    let command = match agent {
        AgentKind::Codex => CommandSpec::new("codex", cwd).with_args(["resume", &session_id]),
        AgentKind::OpenCode => {
            CommandSpec::new("opencode", cwd).with_args(["--session", &session_id])
        }
    };
    println!("{}", serde_json::to_string(&command)?);
    Ok(())
}
