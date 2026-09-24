# Changelog

本文件记录 `zellij-ai-session` 的重要变更。

## [0.3.1] - 2026-09-24

### Added

- 将会话详情中的近期消息预览扩展到所有已支持的 Agent：Claude Code、Cursor、Pi、Reasonix、Codewhale、Qwen Code 和 Goose；Cursor 的本地记录仅包含用户提示，不显示 Agent 回复。
- 在工作台中按 `R` 可通过工具原生接口重命名 Codex、OpenCode、Claude Code、Pi 和 Codewhale 会话。OpenCode 和 Codewhale 需要连接各自运行中的本机服务；未接入原生改名的工具会提示不可用。

### Changed

- 改名后重新扫描原生会话，并在工作台读回新标题后才提示成功；仍可扫描的原生会话以原生标题为准，不再使用工作台此前保存的本地标题覆盖。若用户在原工具中改名，刷新后也会显示原生新标题。
- 更新 Reasonix 的会话路径说明，包含项目目录下的现行存储路径和旧版路径。

### Limitations

- OpenCode 改名仅支持索引器使用默认 XDG 数据目录中的 `opencode.db`，不支持通过 `--opencode-db` 指定的自定义数据库。需先启动使用同一数据目录的本机 `opencode serve`，在 Zellij 环境设置 `OPENCODE_SERVER_URL`（仅接受 `http://127.0.0.1:PORT` 或 `http://localhost:PORT`）；若服务启用了 `OPENCODE_SERVER_PASSWORD`，Zellij 也须提供相同密码。

## [0.3.0] - 2026-09-24

### Added

- 新增跨工具会话工作台：将 Codex、Claude Code、OpenCode、Pi 等工具扫描到的原生会话按项目和父子关系组织，并在 SQLite 中保存项目、标题覆盖、工作目录及会话关系等工作台信息。
- 新增手动整理会话树的能力，可将同一项目中的已有会话设为父子会话，或清除父子关系；不依赖目录、时间或进程信息猜测关系。
- 在 `Alt-S` 会话浮窗中增加 Codex 与 OpenCode 原生对话预览，按原生会话 ID 只读获取最近消息，并显示会话元数据及不可用原因。
- 增加会话树导航操作：空格切换节点展开状态，`C`/`E` 折叠或展开项目，`g`/`G`/`M` 跳转到可见树的首项、末项或中位项。
- 改善会话详情栏的可用宽度，并为长列表导航和会话预览补充阶段记录。

### Scope

- 继续由各 CLI 创建、保存和恢复原生会话；工作台扫描后由用户手动指定父会话。
- 本版本不包含工作台内创建会话、自动捕获新会话 ID、自动交接消息，以及阶段 4 原计划的精确运行 pane 绑定增强。
- 其他工具既有的会话浏览与恢复能力保留；原生消息预览当前支持 Codex 和 OpenCode。
