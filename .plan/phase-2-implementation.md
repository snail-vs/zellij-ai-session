# 阶段 2：会话树与手动整理

日期：2026-09-23。依据 [需求与分阶段计划](session-workbench-requirements.md)、[阶段 0 验证](phase-0-validation.md) 和 [阶段 1 实施](phase-1-implementation.md)。

## 改动

- Alt-S 浮窗直接显示项目与会话树。项目和有子会话的会话可用左右键折叠、展开；上下键移动，回车在项目上切换展开状态，在会话上沿用原有打开路径。树使用 Phase 1 的合并快照，所以未保存的扫描会话是根节点，已保存的父子关系按项目展示。
- `p` 打开项目表单，预填选中项目的主目录或选中会话的实际目录，名称和目录均可编辑；`m` 为选中会话从同项目现有会话中选择父会话，含“无父会话”选项。写入后重新扫描树。
- CLI 增加 `create-project --name NAME --root PATH [--workbench-db PATH]` 与 `set-parent --id SESSION_ID [--parent-id SESSION_ID] [--workbench-db PATH]`。后者只接受当前合并快照中存在的会话，按需补齐稀疏父、子记录，并由数据层拒绝跨项目和循环关系。
- 移除了浮窗中旧的无关联 `n` 新建入口；完整创建和交接属于编号 3 阶段。

## 验证

- `cargo test -q --workspace` 通过；`cargo check -p zellij-ai-session-plugin --features wasm --target wasm32-wasip1` 通过；`git diff --check` 通过。
- 隔离数据库烟测：创建手动项目并重扫可见；选两个同项目原生会话设父关系，重扫后关系仍在；反向设置造成循环时命令失败且原关系保持。
- 尚未在真实 Alt-S 浮窗中进行交互验收。主机目标编译插件时，vendored `zellij-utils` 缺少内嵌的插件 WASM 资源，故使用项目实际部署的 WASI 目标验证。

## 下一阶段接口与风险

- 编号 3 可继续使用 `WorkbenchStore::save_pending` / `resolve_pending`，并通过新增 CLI 命令给插件提供写入与确认入口。创建流程要在启动前保存待处理记录，不能直接复用已移除的旧 `new` UI。
- 项目表单的无选中项默认目录来自插件进程环境；尚未可靠取得唤起 Alt-S 的 CLI pane 目录。阶段 0 留下的来源 pane 验证仍需完成。
- 运行 pane 的识别与切换仍采用旧启发式，不能当作准确会话绑定；编号 4 必须修复。Codex/OpenCode 起始交接和真实原生 ID 的动态验证仍留在编号 3。
