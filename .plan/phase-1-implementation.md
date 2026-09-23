# 阶段 1：持久化与合并

日期：2026-09-23。依据 [需求与分阶段计划](session-workbench-requirements.md) 和 [阶段 0 验证](phase-0-validation.md)。本阶段完成数据层；项目会话树与手动整理交互属于阶段 2。

## 改动与接口

- Codex 索引器按 `session_meta.payload.session_id` 聚合分页文件；缺失时回退旧格式 `id` 并产生扫描警告。标题优先使用原生 `session_index.jsonl` 的线程名称；否则取最早页的用户消息。创建时间取最早页，更新时间取最晚页，实际目录取最早页。异常 ID 不再从文件名猜测。单个损坏文件产生警告，其余文件继续扫描。
- 新增 `WorkbenchStore`，默认数据库位置为 `$XDG_DATA_HOME/zellij-ai-session/workbench.db`，未设置时为 `~/.local/share/zellij-ai-session/workbench.db`。CLI 扫描支持 `scan --workbench-db PATH`，便于隔离验证。SQLite `user_version=1` 创建 `project`、稀疏 `session`、`pending_launch` 三张表；原生历史和消息不写入数据库。
- 项目使用数据库生成的稳定 ID，规范化主目录唯一；扫描时仅为相同主目录创建或复用自动项目。手动创建可将同目录自动项目转换为手动项目。`WorkbenchStore` 提供项目创建/重命名、会话读写、父关系设置、待创建记录读写与事务性完成、扫描合并等 Rust 接口。
- 合并快照保留保存过的项目和父关系，用户标题覆盖原生标题。已保存但暂时无法扫描到的会话仍可见，`native_available=false`；现有导航显示标记并拒绝直接恢复。`selected_cwd` 与原生扫描目录分开保存。同项目及无环约束在写入父关系时校验。

## 验证

- `cargo fmt --all`；`cargo test -q --workspace` 全部通过。
- 新测试覆盖无主分页文件且乱序遍历时的去重与标题、数据库关闭重开后的标题/父子关系/待处理记录、原生记录缺失、跨项目父关系与循环拒绝。
- 使用隔离数据库运行 `zellij-ai-session-index scan --workbench-db /tmp/zellij-ai-session-phase1-smoke.db`：本机 Codex 115 条扫描结果对应 115 个唯一原生 ID，警告 0。阶段 0 记录的 114 个 ID 是此前时间点的样本。

## 下一阶段接口与风险

- 阶段 2 可读取合并后的 `IndexSnapshot.projects/sessions`，其中 `Project.id` 是稳定数据库 ID，`AiSession.parent_id` 表示树关系，`native_available` 表示原生会话是否可恢复。写入可用 `WorkbenchStore`；由于 Zellij 插件运行在 WASM 中，阶段 2 需要通过 indexer CLI 暴露项目创建及父关系命令供插件调用。
- `ensure_session` 可在整理既有扫描会话时补齐稀疏记录；先补齐父、子会话后用 `set_parent` 设置关系。运行 pane 仍沿用现有启发式识别，不能把它当成已确认的原生会话绑定；阶段 4 必须修复。
- 阶段 0 尚未完成的 Codex/OpenCode 起始消息、真实 ID 与 Zellij pane 动态验证仍保留在阶段 3/4/5 的验证范围。
