# zellij-ai-session

> A unified navigator for Zellij to browse, search, resume and manage AI coding-agent sessions (Codex, OpenCode, Claude Code, Qwen Code, Goose, and more) — with one-click install.

[中文文档](README.zh-CN.md)

## Supported Agent TUIs

The navigator discovers past sessions from each agent's local store, groups them by project directory, and resumes them with the agent's own CLI. Eight agents are supported out of the box:

| Agent | Session storage | Resume command |
| --- | --- | --- |
| [Codex](https://github.com/openai/codex) | `~/.codex/sessions/` (index `~/.codex/session_index.jsonl`) | `codex resume <id>` |
| [OpenCode](https://opencode.ai) | `~/.local/share/opencode/opencode.db` (XDG) | `opencode --session <id>` |
| Pi | `~/.pi/agent/sessions/` | `pi --session <path>` |
| Reasonix | `~/.reasonix/sessions/` | `reasonix --resume <path>` |
| Codewhale | `~/.codewhale/sessions/` (legacy `~/.deepseek/sessions/`) | `codewhale --resume <id>` |
| [Claude Code](https://claude.com/claude-code) | `~/.claude/projects/` | `claude --resume <id>` |
| [Qwen Code](https://github.com/QwenLM/qwen-code) | `~/.qwen/projects/` | `qwen --resume <id>` |
| [Goose](https://block.github.io/goose/) | `~/.local/share/goose/sessions/sessions.db` (XDG) | `goose session --resume --session-id <id>` |

Environment overrides:

- Codewhale: `CODEWHALE_HOME`
- Reasonix: `REASONIX_HOME` / `REASONIX_STATE_HOME`
- Claude Code: `CLAUDE_CONFIG_DIR`
- Qwen Code: `QWEN_CONFIG_DIR`
- OpenCode / Goose: `XDG_DATA_HOME`

> Agents whose sessions cannot be mapped to a project directory (e.g. Gemini CLI stores only an irreversible project hash, Aider keeps a per-repo history file, Amazon Q persists state without a documented session list) are intentionally **not** supported, since this tool is directory-first.

Adding a new agent is a two-place change: one `AgentMeta` row in `crates/core/src/lib.rs` plus one `AgentAdapter` implementation (see `crates/indexer/src/`).

## One-line Install

End users need no Rust toolchain — just run:

```bash
curl -fsSL https://raw.githubusercontent.com/snail-vs/zellij-ai-session/main/install.sh | bash
```

The script detects OS/CPU, downloads the prebuilt indexer and WASI plugin from GitHub Release, and verifies `SHA256SUMS`.

Developers building from source run:

```bash
./install.sh --from-source
```

The installer automatically:

- downloads or builds the indexer and Zellij WASI plugin into the user directory;
- creates or updates `~/.config/zellij/config.kdl`;
- binds `Alt s` to open/focus AI Sessions;
- restores sessions in a new tab by default and disables plugin cache so upgrades apply immediately;
- backs up existing config to `.bak.zellij-ai-session` before editing.

Restart Zellij (or reload config) afterwards, then press `Alt s`.

Common options:

```bash
./install.sh --open-mode pane        # restore Session into a pane
./install.sh --key "Ctrl g"          # use a different key binding
./install.sh --version v0.1.0        # install a specific Release
./install.sh --no-keybind            # build & install only, don't touch Zellij config
```

Uninstall:

```bash
./uninstall.sh
```

This removes the installed indexer, WASI plugin and keybinding; config backups are kept and your other Zellij settings are untouched.

Default install locations:

```text
~/.local/bin/zellij-ai-session-index
~/.local/share/zellij-ai-session/zellij_ai_session_plugin.wasm
~/.config/zellij/config.kdl
```

Override these with `ZELLIJ_AI_SESSION_BIN_DIR`, `ZELLIJ_AI_SESSION_DATA_DIR`, `ZELLIJ_AI_SESSION_CONFIG_FILE`, and point the Release source/version with `ZELLIJ_AI_SESSION_REPO` / `ZELLIJ_AI_SESSION_VERSION`.

See [docs/distribution.md](docs/distribution.md) for the GitHub Release and prebuilt-binary design.

## Usage

Navigator keybindings:

```text
Enter   open or resume a Session
n       new Session, then pick an Agent
x       kill the runtime, keep history
/       search
r       manual refresh
q       close the Navigator
```

`Enter` focuses an already-running runtime, or resumes a historical session in a tab/pane per `open_mode`.

Titles prefer each agent's own label: Codex uses the latest `thread_name` from `~/.codex/session_index.jsonl`; OpenCode uses the SQLite `session.title`; Pi uses `session_info.name` (falling back to the first user message); Reasonix uses the `.meta.json` sidecar's `TopicTitle`/`Preview`; Claude Code / Qwen Code use the first user prompt; Goose uses the session `name` or first user message; Codewhale uses `metadata.title`. A default name is used only when no original title exists.

Press `/` to search sessions across all projects. The search covers title, project, directory and agent name, and supports Unicode substrings such as Chinese. Conversation bodies are not searched in the current version.

## Manual Build

The install script covers almost everything. To build by hand:

```bash
cargo build -p zellij-ai-session-index --release
cargo build -p zellij-ai-session-plugin \
  --target wasm32-wasip1 \
  --features wasm \
  --release
```

The plugin is configured via the `indexer` and `open_mode` parameters in your Zellij config:

```kdl
LaunchOrFocusPlugin "file:/absolute/path/to/zellij_ai_session_plugin.wasm" {
    floating true
    move_to_focused_tab true
    skip_plugin_cache true
    indexer "/absolute/path/to/zellij-ai-session-index"
    open_mode "tab"
}
```

## Verify

```bash
cargo fmt --all --check
cargo test --workspace
cargo check -p zellij-ai-session-plugin \
  --target wasm32-wasip1 \
  --features wasm
```
