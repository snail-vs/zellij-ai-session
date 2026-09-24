use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

struct ManagedChild(Child);

impl Drop for ManagedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub(super) fn rename_codex(session_id: &str, title: &str) -> Result<()> {
    let mut child = ManagedChild(
        Command::new("codex")
            .args(["app-server", "--stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("start Codex app-server")?,
    );
    let mut stdin = child
        .0
        .stdin
        .take()
        .context("Codex app-server stdin unavailable")?;
    let stdout = child
        .0
        .stdout
        .take()
        .context("Codex app-server stdout unavailable")?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let value = line.map_err(|error| error.to_string()).and_then(|line| {
                serde_json::from_str::<Value>(&line).map_err(|error| error.to_string())
            });
            if tx.send(value).is_err() {
                break;
            }
        }
    });
    rpc_send(
        &mut stdin,
        &json!({"id":1,"method":"initialize","params":{
            "clientInfo":{"name":"zellij-ai-session","title":"Zellij AI Session","version":"0.1.0"},
            "capabilities":{}
        }}),
    )?;
    rpc_result(&rx, 1)?;
    rpc_send(&mut stdin, &json!({"method":"initialized","params":{}}))?;
    rpc_send(
        &mut stdin,
        &json!({"id":2,"method":"thread/name/set","params":{
            "threadId":session_id,"name":title
        }}),
    )?;
    rpc_result(&rx, 2)?;
    Ok(())
}

fn rpc_send(stdin: &mut impl Write, value: &Value) -> Result<()> {
    serde_json::to_writer(&mut *stdin, value)?;
    stdin.write_all(b"\n")?;
    stdin.flush()?;
    Ok(())
}

fn rpc_result(rx: &Receiver<std::result::Result<Value, String>>, id: u64) -> Result<()> {
    loop {
        let value = rx
            .recv_timeout(Duration::from_secs(10))
            .with_context(|| format!("Codex app-server did not respond to request {id}"))?
            .map_err(anyhow::Error::msg)?;
        if value.get("id").and_then(Value::as_u64) != Some(id) {
            continue;
        }
        if let Some(error) = value.get("error") {
            bail!("Codex rename failed: {error}");
        }
        if value.get("result").is_none() {
            bail!("Codex app-server returned no result for request {id}");
        }
        return Ok(());
    }
}

pub(super) fn rename_opencode(session_id: &str, title: &str) -> Result<()> {
    let base = std::env::var("OPENCODE_SERVER_URL").context(
        "OpenCode native renaming requires OPENCODE_SERVER_URL for a running local server",
    )?;
    let base = base.trim_end_matches('/');
    let port = base
        .strip_prefix("http://127.0.0.1:")
        .or_else(|| base.strip_prefix("http://localhost:"))
        .and_then(|port| port.parse::<u16>().ok());
    anyhow::ensure!(
        port.is_some_and(|port| port != 0),
        "OpenCode server URL must be http://127.0.0.1:PORT or http://localhost:PORT"
    );
    let body = json!({"title":title}).to_string();
    let mut command = Command::new("curl");
    command.args([
        "--silent",
        "--show-error",
        "--noproxy",
        "*",
        "--fail-with-body",
        "--max-time",
        "30",
        "-X",
        "PATCH",
        "-H",
        "Content-Type: application/json",
        "--data-raw",
        &body,
    ]);
    if let Ok(password) = std::env::var("OPENCODE_SERVER_PASSWORD") {
        command.args(["--user", &format!("opencode:{password}")]);
    }
    let output = command
        .arg(format!("{base}/session/{session_id}"))
        .output()
        .context("call OpenCode rename API")?;
    if !output.status.success() {
        bail!(
            "OpenCode rename failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

pub(super) fn rename_codewhale(session_id: &str, title: &str) -> Result<()> {
    let base = std::env::var("CODEWHALE_SERVER_URL").context(
        "Codewhale native renaming requires CODEWHALE_SERVER_URL for a running local Runtime API",
    )?;
    let base = base.trim_end_matches('/');
    let port = base
        .strip_prefix("http://127.0.0.1:")
        .or_else(|| base.strip_prefix("http://localhost:"))
        .and_then(|port| port.parse::<u16>().ok());
    anyhow::ensure!(
        port.is_some_and(|port| port != 0),
        "Codewhale server URL must be http://127.0.0.1:PORT or http://localhost:PORT"
    );
    let body = json!({"title":title}).to_string();
    let mut command = Command::new("curl");
    command.args([
        "--silent",
        "--show-error",
        "--noproxy",
        "*",
        "--fail-with-body",
        "--max-time",
        "30",
        "-X",
        "PATCH",
        "-H",
        "Content-Type: application/json",
        "--data-raw",
        &body,
    ]);
    if let Ok(token) = std::env::var("CODEWHALE_RUNTIME_TOKEN") {
        command.args(["-H", &format!("Authorization: Bearer {token}")]);
    }
    let output = command
        .arg(format!("{base}/v1/sessions/{session_id}"))
        .output()
        .context("call Codewhale rename API")?;
    if !output.status.success() {
        bail!(
            "Codewhale rename failed: {} {}",
            String::from_utf8_lossy(&output.stderr).trim(),
            String::from_utf8_lossy(&output.stdout).trim()
        );
    }
    Ok(())
}
