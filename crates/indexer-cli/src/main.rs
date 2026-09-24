use std::env;

use anyhow::Result;
use zellij_ai_session_core::{AgentKind, CommandSpec, ProjectOrigin};
use zellij_ai_session_indexer::{Indexer, IndexerConfig, workbench::WorkbenchStore};

fn main() -> Result<()> {
    let mut config = IndexerConfig::default();
    let mut workbench_db = None;
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "scan".into());
    if command == "resume" {
        return resume_command(args.collect());
    }
    if command == "new" {
        return new_command(args.collect());
    }
    if command == "create-project" {
        return create_project_command(args.collect());
    }
    if command == "set-parent" {
        return set_parent_command(args.collect());
    }
    if command == "preview" {
        return preview_command(args.collect());
    }
    if command == "rename-session" {
        return rename_session_command(args.collect());
    }

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--codex-home" => config.codex_home = args.next().map(Into::into),
            "--opencode-db" => config.opencode_db = args.next().map(Into::into),
            "--cursor-home" => config.cursor_home = args.next().map(Into::into),
            "--workbench-db" => workbench_db = args.next().map(Into::into),
            "--help" | "-h" => {
                println!(
                    "zellij-ai-session-index [scan|resume|new|create-project|set-parent|preview|rename-session] [options]"
                );
                return Ok(());
            }
            unknown => anyhow::bail!("unknown argument: {unknown}"),
        }
    }

    let snapshot = Indexer::from_config(config).scan();
    let db_path = workbench_db.unwrap_or(WorkbenchStore::default_path()?);
    let snapshot = WorkbenchStore::open(&db_path)?.merge(snapshot)?;
    println!("{}", serde_json::to_string_pretty(&snapshot)?);
    Ok(())
}

fn rename_session_command(args: Vec<String>) -> Result<()> {
    let mut id = None;
    let mut title = None;
    let mut config = IndexerConfig::default();
    let mut workbench_db = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--id" => id = args.next(),
            "--title" => title = args.next(),
            "--codex-home" => config.codex_home = args.next().map(Into::into),
            "--opencode-db" => config.opencode_db = args.next().map(Into::into),
            "--workbench-db" => workbench_db = args.next().map(Into::into),
            unknown => anyhow::bail!("unknown rename-session argument: {unknown}"),
        }
    }
    let id = id.ok_or_else(|| anyhow::anyhow!("missing --id"))?;
    let title = title.ok_or_else(|| anyhow::anyhow!("missing --title"))?;
    let (agent_name, native_id) = id
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("invalid session ID: {id}"))?;
    let agent = AgentKind::from_command_name(agent_name)
        .ok_or_else(|| anyhow::anyhow!("unsupported agent: {agent_name}"))?;
    let indexer = Indexer::from_config(config);
    let renamed = indexer.rename_session(agent, native_id, &title)?;
    let db_path = workbench_db.unwrap_or(WorkbenchStore::default_path()?);
    let mut store = WorkbenchStore::open(&db_path)?;
    store.clear_title_override(&id)?;
    let after = store.merge(indexer.scan())?;
    let visible = after
        .sessions
        .iter()
        .find(|session| session.id == id)
        .ok_or_else(|| anyhow::anyhow!("renamed session disappeared from workbench"))?;
    anyhow::ensure!(
        visible.title == renamed.title,
        "native rename succeeded but workbench still displays a different title; check saved metadata"
    );
    println!("{}", serde_json::to_string(visible)?);
    Ok(())
}

fn preview_command(args: Vec<String>) -> Result<()> {
    let mut agent = None;
    let mut session_id = None;
    let mut config = IndexerConfig::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--agent" => agent = args.next(),
            "--session-id" => session_id = args.next(),
            "--codex-home" => config.codex_home = args.next().map(Into::into),
            "--opencode-db" => config.opencode_db = args.next().map(Into::into),
            unknown => anyhow::bail!("unknown preview argument: {unknown}"),
        }
    }
    let agent = agent.ok_or_else(|| anyhow::anyhow!("missing --agent"))?;
    let agent = AgentKind::from_command_name(&agent)
        .ok_or_else(|| anyhow::anyhow!("unsupported agent: {agent}"))?;
    let session_id = session_id.ok_or_else(|| anyhow::anyhow!("missing --session-id"))?;
    println!(
        "{}",
        serde_json::to_string(&Indexer::from_config(config).preview(agent, &session_id)?)?
    );
    Ok(())
}

fn create_project_command(args: Vec<String>) -> Result<()> {
    let mut name = None;
    let mut root = None;
    let mut db = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--name" => name = args.next(),
            "--root" => root = args.next(),
            "--workbench-db" => db = args.next(),
            unknown => anyhow::bail!("unknown create-project argument: {unknown}"),
        }
    }
    let name = name.ok_or_else(|| anyhow::anyhow!("missing --name"))?;
    let root: std::path::PathBuf = root
        .ok_or_else(|| anyhow::anyhow!("missing --root"))?
        .into();
    if !root.is_dir() {
        anyhow::bail!("project directory does not exist: {}", root.display());
    }
    let path = match db {
        Some(path) => path.into(),
        None => WorkbenchStore::default_path()?,
    };
    let mut store = WorkbenchStore::open(&path)?;
    let project = store.create_project(&name, &root, ProjectOrigin::Manual)?;
    println!("{}", serde_json::to_string(&project)?);
    Ok(())
}

fn set_parent_command(args: Vec<String>) -> Result<()> {
    let mut id = None;
    let mut parent_id = None;
    let mut db = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--id" => id = args.next(),
            "--parent-id" => parent_id = args.next(),
            "--workbench-db" => db = args.next(),
            unknown => anyhow::bail!("unknown set-parent argument: {unknown}"),
        }
    }
    let id = id.ok_or_else(|| anyhow::anyhow!("missing --id"))?;
    let path = match db {
        Some(path) => path.into(),
        None => WorkbenchStore::default_path()?,
    };
    let mut store = WorkbenchStore::open(&path)?;
    let snapshot = store.merge(Indexer::from_config(IndexerConfig::default()).scan())?;
    let child = snapshot
        .sessions
        .iter()
        .find(|session| session.id == id)
        .ok_or_else(|| anyhow::anyhow!("session {id} was not found"))?;
    let parent = parent_id
        .as_deref()
        .map(|parent_id| {
            snapshot
                .sessions
                .iter()
                .find(|session| session.id == parent_id)
                .ok_or_else(|| anyhow::anyhow!("parent session {parent_id} was not found"))
        })
        .transpose()?;
    if let Some(parent) = parent {
        if parent.project_id != child.project_id {
            anyhow::bail!("parent and child must belong to the same project");
        }
        store.ensure_session(parent, &parent.project_id)?;
    }
    store.ensure_session(child, &child.project_id)?;
    store.set_parent(&id, parent_id.as_deref())?;
    println!("{{\"ok\":true}}");
    Ok(())
}

fn new_command(args: Vec<String>) -> Result<()> {
    let (agent, cwd) = parse_agent_and_cwd(args, "new")?;
    let command = CommandSpec::new(agent.command_name(), cwd);
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
                println!("zellij-ai-session-index {command_name} --agent <name> --cwd PATH");
                return Err(anyhow::anyhow!("help requested"));
            }
            unknown => anyhow::bail!("unknown {command_name} argument: {unknown}"),
        }
    }
    let agent = match agent.as_deref() {
        Some(name) => AgentKind::from_command_name(name)
            .ok_or_else(|| anyhow::anyhow!("unsupported agent: {name}"))?,
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
                println!("zellij-ai-session-index resume --agent NAME --session-id ID --cwd PATH");
                return Ok(());
            }
            unknown => anyhow::bail!("unknown resume argument: {unknown}"),
        }
    }

    let agent = agent.ok_or_else(|| anyhow::anyhow!("missing --agent"))?;
    let session_id = session_id.ok_or_else(|| anyhow::anyhow!("missing --session-id"))?;
    let cwd = cwd.ok_or_else(|| anyhow::anyhow!("missing --cwd"))?.into();
    let agent = AgentKind::from_command_name(&agent)
        .ok_or_else(|| anyhow::anyhow!("unsupported agent: {agent}"))?;
    let mut args: Vec<String> = agent
        .resume_args()
        .iter()
        .map(|arg| arg.to_string())
        .collect();
    args.push(session_id);
    let command = CommandSpec::new(agent.command_name(), cwd).with_args(args);
    println!("{}", serde_json::to_string(&command)?);
    Ok(())
}
