# zellij-ai-session

> Find and resume any AI coding session by project, directly inside Zellij.

[![CI](https://github.com/snail-vs/zellij-ai-session/actions/workflows/ci.yml/badge.svg)](https://github.com/snail-vs/zellij-ai-session/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/snail-vs/zellij-ai-session)](https://github.com/snail-vs/zellij-ai-session/releases/latest)
[![License](https://img.shields.io/github/license/snail-vs/zellij-ai-session)](LICENSE)

[中文文档](README.zh-CN.md)

![Open zellij-ai-session, select a project, search, and resume a session](docs/demo.gif)

Codex, OpenCode, Claude Code and other agent CLIs each keep their own session history. `zellij-ai-session` discovers those existing local sessions, groups them by project directory, and gives you one place to search and continue the work.

It does not replace your agents or migrate their history. Press `Alt s`, choose the project, and press `Enter`; the navigator focuses a running session or resumes a historical one with the original agent CLI.

## Highlights

- **Project-first:** work is grouped by directory, even when one project uses several agents.
- **Multi-agent:** nine agent session formats are supported out of the box.
- **One action to continue:** the same `Enter` action focuses a running session or resumes history.
- **Local by design:** session discovery and search run on your machine; no account or cloud service is required.
- **Ready-to-install binaries:** Linux and macOS users do not need a Rust toolchain.

## Quick Start

Prerequisites:

- Zellij installed and available on `PATH`;
- at least one supported agent CLI with an existing session.

Install the latest release:

```bash
curl -fsSL https://raw.githubusercontent.com/snail-vs/zellij-ai-session/main/install.sh | bash
```

The installer downloads prebuilt release assets, verifies `SHA256SUMS`, and configures `Alt s`. Restart Zellij or reload its configuration, then press `Alt s`.

If you prefer to inspect the installer before running it:

```bash
curl -fsSL https://raw.githubusercontent.com/snail-vs/zellij-ai-session/main/install.sh \
  -o /tmp/zellij-ai-session-install.sh
less /tmp/zellij-ai-session-install.sh
bash /tmp/zellij-ai-session-install.sh --version latest
```

The installer:

- installs the native indexer and Zellij WASI plugin in the current user's directories;
- creates or updates `~/.config/zellij/config.kdl`;
- binds `Alt s` to open or focus the navigator;
- opens resumed sessions in a new tab by default;
- backs up an existing config as `.bak.zellij-ai-session` before changing it.

Common options:

```bash
./install.sh --open-mode pane        # resume into a pane instead of a tab
./install.sh --key "Ctrl g"          # use a different key binding
./install.sh --version latest       # install the latest release
./install.sh --no-keybind            # install files without editing Zellij config
./install.sh --from-source           # build locally; requires Rust
```

Uninstall:

```bash
./uninstall.sh
```

Uninstall removes the installed indexer, WASI plugin and managed keybinding. It keeps configuration backups and does not remove agent history.

## Usage

```text
Enter   open or resume a session
n       create a session, then choose an agent
x       close the runtime, keep agent history
/       search all discovered sessions
r       refresh the index
q       close the navigator
```

`Enter` focuses a matching Zellij runtime when one exists. Otherwise it starts the agent's native resume command in a tab or pane according to `open_mode`.

Search covers session title, project, directory and agent name, including Unicode substrings such as Chinese text.

## Supported Agents

| Agent | Local session store | Resume command |
| --- | --- | --- |
| [Codex](https://github.com/openai/codex) | `~/.codex/sessions/` and `~/.codex/session_index.jsonl` | `codex resume <id>` |
| [OpenCode](https://opencode.ai) | `~/.local/share/opencode/opencode.db` (XDG) | `opencode --session <id>` |
| [Cursor](https://cursor.com) | `~/.cursor/chats/<workspace>/<chat>/meta.json` | `cursor-agent --resume <id>` |
| Pi | `~/.pi/agent/sessions/` | `pi --session <path>` |
| Reasonix | `~/.reasonix/sessions/` | `reasonix --resume <path>` |
| Codewhale | `~/.codewhale/sessions/` or legacy `~/.deepseek/sessions/` | `codewhale --resume <id>` |
| [Claude Code](https://claude.com/claude-code) | `~/.claude/projects/` | `claude --resume <id>` |
| [Qwen Code](https://github.com/QwenLM/qwen-code) | `~/.qwen/projects/` | `qwen --resume <id>` |
| [Goose](https://block.github.io/goose/) | `~/.local/share/goose/sessions/sessions.db` (XDG) | `goose session --resume --session-id <id>` |

Environment overrides:

- Codewhale: `CODEWHALE_HOME`
- Reasonix: `REASONIX_HOME` / `REASONIX_STATE_HOME`
- Claude Code: `CLAUDE_CONFIG_DIR`
- Qwen Code: `QWEN_CONFIG_DIR`
- Cursor: `CURSOR_CONFIG_DIR` (the directory containing `chats/`)
- OpenCode / Goose: `XDG_DATA_HOME`

Agents that cannot reliably map stored history back to a project directory are intentionally excluded from the default registry. New agents can be proposed with the [agent request template](https://github.com/snail-vs/zellij-ai-session/issues/new/choose).

## Privacy and Safety

- The indexer reads supported agents' local session metadata. Some adapters inspect the first user message when an agent does not provide a session title.
- Session data is processed locally. The application has no telemetry and does not upload prompts or history.
- The installer uses the network only to retrieve release metadata, binaries and checksums from GitHub.
- Resuming a session launches the corresponding agent CLI with that agent's normal permissions and configuration.
- Pressing `x` closes the matching Zellij runtime; it does not delete the agent's stored session history.
- Before editing Zellij configuration, the installer creates a backup. Use `--no-keybind` when you want to configure the plugin manually.

When reporting a problem, do not attach raw session files or prompts. Redact usernames, home-directory paths, repository names and secrets from logs or configuration snippets.

## Compatibility

Release binaries are published for:

- Linux: `x86_64`, `aarch64`
- macOS: Intel `x86_64`, Apple Silicon `aarch64`

The current plugin is built against the Zellij `0.44.3` plugin API. If you encounter a compatibility problem, open a bug report with the Zellij version, operating system, architecture and affected agent.

## Configuration

Default install locations:

```text
~/.local/bin/zellij-ai-session-index
~/.local/share/zellij-ai-session/zellij_ai_session_plugin.wasm
~/.config/zellij/config.kdl
```

Override these with `ZELLIJ_AI_SESSION_BIN_DIR`, `ZELLIJ_AI_SESSION_DATA_DIR` and `ZELLIJ_AI_SESSION_CONFIG_FILE`. Use `ZELLIJ_AI_SESSION_REPO` and `ZELLIJ_AI_SESSION_VERSION` to select the release source and version.

Manual Zellij configuration:

```kdl
LaunchOrFocusPlugin "file:/absolute/path/to/zellij_ai_session_plugin.wasm" {
    floating true
    move_to_focused_tab true
    skip_plugin_cache true
    indexer "/absolute/path/to/zellij-ai-session-index"
    open_mode "tab"
}
```

See [docs/distribution.md](docs/distribution.md) for the release artifact design.

## Development

Build from source:

```bash
cargo build -p zellij-ai-session-index --release
cargo build -p zellij-ai-session-plugin \
  --target wasm32-wasip1 \
  --features wasm \
  --release
```

Verify changes:

```bash
cargo fmt --all --check
cargo test --workspace
cargo check -p zellij-ai-session-plugin \
  --target wasm32-wasip1 \
  --features wasm
```

Adding an agent requires one `AgentMeta` registry row in `crates/core/src/lib.rs` and one `AgentAdapter` implementation under `crates/indexer/src/`. See [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request.

## License

[MIT](LICENSE)
