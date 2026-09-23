# 阶段 0：身份与运行验证记录

日期：2026-09-23。依据：[需求与分阶段计划](session-workbench-requirements.md)，本阶段只调查并确定接口边界，不实现后续阶段。

**本阶段产物**：本验证记录及其中的阶段 1 接口约束。当前完成了静态和只读验证；新建会话、起始消息、真实 pane 绑定和浮窗交互尚未实测，不能把本记录当作动态闭环验收通过。

## 环境与验证范围

- 仓库工作树起始状态：只有 `.plan/` 未跟踪；没有修改已有代码。已安装 `codex-cli 0.156.1`、`opencode 1.18.32`、`zellij 0.45.1`。
- 验证方式：CLI `--help`、现有原生存储的只读结构检查、现有索引器扫描、当前 Zellij 的只读 pane 查询，以及仓库源码检查。未发送模型请求，未创建新的 Codex/OpenCode 原生会话，未对用户当前 Zellij pane 执行聚焦或调整尺寸。因此“启动后首次持久化时机”和实际 Alt-S 浮窗调整效果仍需交互测试，不能写成已验证通过。
- 沙箱内 `zellij action list-panes` 无法连接现有会话，实际 pane 查询在获准的沙箱外以只读方式执行；对用户数据仅输出汇总字段，不输出会话内容。

## 逐工具结果

| 工具 | 已验证的原生 ID 来源与恢复方式 | 新建与交接入口 | 尚需动态验证 |
| --- | --- | --- | --- |
| Codex | 现有 `~/.codex/sessions/**/*.jsonl` 的 `session_meta.payload` 同时有 `id`、`session_id`、`cwd`。分页历史中，`id` 是 rollout/page ID，`session_id` 是恢复时使用的 thread ID；`codex resume <session_id>` 按 thread ID 恢复。索引器优先取 `session_id` 构造 `codex:<ID>`，但目前没有按它去重。 | `codex [PROMPT]` 的帮助显示交接文本可作交互会话起始 prompt；`codex resume <ID> [PROMPT]` 也接受 prompt。索引器现有 `new` 只返回 `codex` 命令和目录，未携带交接文本，也不捕获 ID。 | 首次启动何时写 `session_meta`，prompt 何时写入 `response_item`，启动失败时是否已有可恢复会话。当前实存记录的用户 `response_item.payload.content` 含 `input_text`，可作为关联标记的候选读取位置，但尚未做新会话端到端验证。 |
| OpenCode | 现有 `~/.local/share/opencode/opencode.db` 的 `session.id` 为 `ses_...`，含 `directory`；`message`、`part` 均有 `session_id`。索引器读取 `session` 表，构造 `opencode:<ID>`；`opencode --session <ID>` 可按 ID 恢复。 | `opencode --prompt <文本>` 的帮助显示交互 TUI 支持起始 prompt；`opencode run [message..]` 是非交互模式，不能直接代替保留原生 TUI 的启动路径。索引器现有 `new` 只返回 `opencode` 命令和目录。 | TUI `--prompt` 产生 `session` 行及 `message`/`part` 的先后时机；失败或退出前是否会留下记录。现有 `part.data` 存在文本类型，可作为关联标记候选，但尚未用新会话确认。 |

复现只读检查：

```sh
codex --version
codex --help
codex resume --help
opencode --version
opencode --help
opencode run --help
opencode db path
target/debug/zellij-ai-session-index new --agent codex --cwd /tmp
target/debug/zellij-ai-session-index new --agent opencode --cwd /tmp
target/debug/zellij-ai-session-index resume --agent codex --session-id test-id --cwd /tmp
target/debug/zellij-ai-session-index resume --agent opencode --session-id ses_test --cwd /tmp
zellij --version
target/debug/zellij-ai-session-index | python3 -c 'import json,sys,collections; j=json.load(sys.stdin); print(dict(collections.Counter(s["agent"] for s in j["sessions"])), len(j["warnings"]))'
target/debug/zellij-ai-session-index | python3 -c 'import json,sys,collections; s=json.load(sys.stdin)["sessions"]; [(lambda a: print(t, len(a), len({x["id"] for x in a})))([x for x in s if x["agent"]==t]) for t in ("codex", "opencode")]'
```

本机索引器扫描结果：Codex 175 行、OpenCode 97 行，另有其他工具会话；`warnings` 为 0。Codex 的 175 份非空 JSONL 只对应 **114 个不同的 `session_id`**：其中 5 个 thread 有多个 rollout/page 文件，共有 61 个分页文件的 `id != session_id`。另有 1 份空 JSONL 未被扫描。OpenCode 的 97 行对应 97 个不同的 `session.id`。Codex/OpenCode 扫描出的 `AiSession.id` 形式虽正确，但 Codex 结果**没有唯一性**，不能直接作为 SQLite 合并输入。两个同工具、同目录的会话始终只能以原生 ID 区分；也不能据扫描时间自动认领新会话。

上述 `new` 命令均输出无参数的工具命令；`resume` 分别输出 `codex resume test-id` 和 `opencode --session ses_test`，只构造命令，不会启动原生 CLI。

`codex migrate-rollouts --help` 明确描述“paginated thread history”。本机分页文件中每个多页 thread 都恰有一份 `id == session_id` 的主文件，且页间 `cwd` 相同；这只是当前样本，去重逻辑不应把“始终有主文件”当不变约束。阶段 1 接入合并前，应按 `session_id` 聚合页，输出每个 thread 一条会话；对标题、最后更新时间和实际目录明确合并规则，并覆盖主文件缺失、页乱序、重复扫描的测试。缺少 `session_id` 的旧格式才考虑以 `id` 回退，并把格式异常报告为扫描警告。

## Zellij pane、浮窗与目录

- `open_command_pane_in_new_tab` 返回 tab/pane ID，`open_command_pane_near_plugin` 返回 pane ID；插件订阅了 `CommandPaneOpened`，该事件带 pane ID 和创建请求 context，但目前没有处理这个事件。创建请求可用 `request_id` 放入 context，得到**本次启动的 pane ID**；它不是原生会话 ID。
- 当前插件 `update_runtime` 用 `PaneInfo.terminal_command` 包含工具名来识别 CLI，再用工具名及工作目录匹配会话；若只有一个候选 pane，会把它分配给所有同工具、同目录的扫描会话。之后 `resume_or_open` 直接 `show_pane_with_id`，可能切错会话。`crates/indexer/src/lib.rs::apply_runtime` 有同类启发式，但当前插件没有调用它。后续阶段不能沿用这一结果作为“准确关联”。
- Zellij 0.45.1 当前会话的 `list-panes --all --json` 中，1 个正在运行的 Codex pane 有 `pane_command=codex` 和 `pane_cwd`，却没有 `terminal_command`。这是从 shell 中启动 CLI 的正常情形：`PaneInfo.terminal_command` 只描述 Zellij *command pane* 的启动命令。当前插件可能完全漏掉这类 pane。`PaneManifest` 也没有 `pane_cwd` 字段；插件已使用 `get_pane_cwd(PaneId)`，前提是先知道准确 pane ID。
- 运行中的 pane ID 在当前 Zellij session 内可用于聚焦；重启或跨 Zellij session 时应重新验证。`show_pane_with_id` 能聚焦已知 pane，但运行时关联需要持有明确的 `(<Zellij session>, <pane ID>, <原生会话键>)`。若没有确认该三元组，显示“运行位置未知”，不可用同目录推测后直接切换。
- 默认安装绑定 `Alt s` 的 `LaunchOrFocusPlugin`，配置 `floating true`、`move_to_focused_tab true`。现有插件以 `render(rows, cols)` 接收浮窗可用尺寸，代码未设置浮窗坐标或大小。Zellij 0.45.1 提供 `action change-floating-pane-coordinates --pane-id ... --width ... --height ...`、`toggle-fullscreen`；CLI 帮助证明接口存在，插件内自动调整的可用 API 与快捷键交互效果仍未验证。
- `get_focused_pane_info()` 和 `get_pane_cwd(PaneId)` 在所依赖的 `zellij-tile` API 中存在。浮窗获得焦点后，当前焦点未必再是唤起它的 CLI；而 `PaneInfo.is_focused` 是按层分别表示，不能把所有 `is_focused=true` 的 pane 都当唤起者。当前 `create_selected_session` 仅取选中项目的 `root_directory`，没有读取唤起 pane 的 cwd。后续需要在打开导航器时记录调用来源 pane，或提供明确选择；没有可靠来源时默认项目主目录，并显示这是默认值。

复现只读查询（在有权连接目标 Zellij session 的环境执行）：

```sh
zellij action list-panes --all --json
zellij action change-floating-pane-coordinates --help
zellij action toggle-fullscreen --help
```

## 后续阶段的接口建议与待处理流程

1. 阶段 1 的持久化键为 `<tool>:<native_session_id>`。`PendingLaunch.request_id` 是创建操作唯一 ID，保存工具、目录、父会话、交接内容/路径及状态；可增加已确认的 `observed_pane` 等诊断字段，但 pane ID 仅是当前运行时线索。存储 `selected_cwd` 与原生扫描所得 `directory` 两个字段，不将二者混为身份。
2. 阶段 3 启动时先提交 `PendingLaunch`，通过 Zellij 创建请求 context 关联 `request_id` 与返回/事件中的 pane ID。起始文本中可放一次性 `request_id` 标记，再从**该原生会话的首条用户消息**与原生存储 ID 关联；这是待动态验证的候选方案。仅凭新增时间、工具、目录、pane 的命令或 PID 均不能确定原生 ID。若 CLI 提供更直接的原生 ID 事件，应优先使用并验证；不能把 `opencode run` 的 JSON 事件推定为交互 TUI 的事件。
3. 未取得唯一原生 ID、首条消息未持久化、启动命令失败、插件重载、Zellij 事件丢失，均保留 `PendingLaunch`。再次发送交接前先检查是否已有带该 `request_id` 的原生消息。查到一个则补齐关联；查到多个或零个但存在候选时展示候选 ID、工具和实际目录，请用户明确选择；仍无法确定则保持待处理，不启动重复交接。阶段 2 的手动父子关联应允许在新会话被扫描到后修复关系。
4. 阶段 4 切换前校验 pane 尚存在，并确认其已记录的原生会话键；无法确认时不能聚焦一个猜测的 pane。原生进程退出后，只用原生 ID 构造恢复命令。修复现有启发式匹配属于阶段 4 的必要前置工作；阶段 1 合并视图应把运行位置视为不可靠的临时数据。

## 下一阶段交接

阶段 1 可直接依赖：原生会话键格式、两工具只读扫描的 `id/directory/title/time`、`selected_cwd` 与实际 `directory` 的区分、待处理记录必须独立于原生会话。**先消除 Codex 多页历史造成的重复会话键，再实现持久化合并。**继续保持原生消息/历史只读且不复制进工作台 SQLite。阶段 1 的合并视图应能标明“原生会话暂未扫描到”；这与“pane 未确认”是两种不同的不可用状态。

仍存在的风险：Codex/OpenCode 的启动交接与原生 ID 绑定、Alt-S 来源 pane 与实际浮窗调整尚未完成交互验证。相关端到端测试必须在阶段 3/4/5 实施前或实施时完成；无法确认时使用上述待处理和手动选择流程，不将本阶段的静态证据扩大解释为成功闭环。
