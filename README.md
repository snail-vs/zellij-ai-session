# zellij-ai-session

Project-first session navigator for Codex and OpenCode in Zellij.

面向 GitHub Release 和预编译产物的发布方案见：[GitHub 发布与一键安装方案](docs/distribution.md)。

## 一键安装

准备好 Rust（推荐通过 [rustup](https://rustup.rs/) 安装）后，在项目目录执行：

```bash
./install.sh
```

安装脚本会自动：

- 安装 `wasm32-wasip1` 编译目标；
- 构建并安装 indexer 和 Zellij WASI 插件到当前用户目录；
- 自动创建或更新 `~/.config/zellij/config.kdl`；
- 配置 `Alt s` 打开或聚焦 AI Sessions；
- 配置默认在新 tab 中恢复 Session，并关闭插件缓存，升级后直接生效；
- 修改已有配置前创建 `.bak.zellij-ai-session` 备份。

安装完成后重启 Zellij（或重新加载配置），按 `Alt s` 即可打开。

常用选项：

```bash
# 恢复 Session 到 pane
./install.sh --open-mode pane

# 使用其他快捷键
./install.sh --key "Ctrl g"

# 只构建和安装文件，不修改 Zellij 配置
./install.sh --no-keybind
```

卸载：

```bash
./uninstall.sh
```

卸载会移除安装的 indexer、WASI 插件和脚本管理的快捷键；配置备份会保留，不会删除用户其他配置。

默认安装位置：

```text
~/.local/bin/zellij-ai-session-index
~/.local/share/zellij-ai-session/zellij_ai_session_plugin.wasm
~/.config/zellij/config.kdl
```

可以通过 `ZELLIJ_AI_SESSION_BIN_DIR`、`ZELLIJ_AI_SESSION_DATA_DIR` 和 `ZELLIJ_AI_SESSION_CONFIG_FILE` 覆盖这些路径。

## 使用方式

Navigator 内快捷键：

```text
Enter  打开或恢复 Session
n      新建 Session 并选择 Agent
x      关闭 runtime，保留历史记录
/      搜索
r      手动刷新
q      关闭 Navigator
```

`Enter` 打开已有 runtime 时会聚焦它；历史 Session 会按 `open_mode` 在 tab 或 pane 中恢复。

## 手工构建

安装脚本已经覆盖绝大多数场景。需要手工构建时：

```bash
cargo build -p zellij-ai-session-index --release
cargo build -p zellij-ai-session-plugin \
  --target wasm32-wasip1 \
  --features wasm \
  --release
```

插件可以通过 Zellij 配置中的 `indexer` 和 `open_mode` 参数配置：

```kdl
LaunchOrFocusPlugin "file:/absolute/path/to/zellij_ai_session_plugin.wasm" {
    floating true
    move_to_focused_tab true
    skip_plugin_cache true
    indexer "/absolute/path/to/zellij-ai-session-index"
    open_mode "tab"
}
```

## 验证

```bash
cargo fmt --all --check
cargo test --workspace
cargo check -p zellij-ai-session-plugin \
  --target wasm32-wasip1 \
  --features wasm
```
