# GitHub 发布与一键安装方案

本文说明如何将 `zellij-ai-session` 发布到 GitHub，让普通用户不需要安装 Rust，也不需要从源码构建即可使用。

## 一、当前架构

项目由两个运行时组件组成：

```text
Zellij WASI 插件
        │
        │ 通过 Zellij run_command 调用
        ▼
zellij-ai-session-index 原生可执行文件
        │
        ├── 扫描 Codex JSONL
        ├── 查询 OpenCode SQLite
        ├── 生成统一的 Session 数据
        └── 生成 resume/new 命令
```

WASI 插件负责 UI 和交互，indexer 负责访问本地文件系统和生成命令。插件不能把原生 indexer 直接塞进 WASM 中运行，因此发布时必须同时提供 WASM 插件和原生 indexer。

## 二、需要发布的产物

### 1. 通用 WASI 插件

插件产物为：

```text
zellij_ai_session_plugin.wasm
```

构建命令：

```bash
cargo build \
  -p zellij-ai-session-plugin \
  --target wasm32-wasip1 \
  --features wasm \
  --release
```

这个文件通常可以在支持对应 Zellij Plugin API 的不同操作系统之间通用。

插件负责：

- 项目、Session 列表渲染；
- 键盘导航和搜索；
- 打开、恢复、关闭 runtime；
- 调用 indexer；
- 在 tab 或 pane 中打开 Agent Session。

### 2. 平台相关的 native indexer

indexer 需要按操作系统和 CPU 架构分别构建，例如：

```text
zellij-ai-session-index-linux-x86_64
zellij-ai-session-index-linux-aarch64
zellij-ai-session-index-macos-x86_64
zellij-ai-session-index-macos-aarch64
```

Linux x86_64 构建命令：

```bash
cargo build -p zellij-ai-session-index --release
```

indexer 负责：

- 读取 Codex Session 数据；
- 查询 OpenCode 数据库；
- 规范化项目目录；
- 识别历史 Session 和运行中的 runtime；
- 生成恢复或新建 Agent Session 所需的命令。

## 三、GitHub Release 目录结构

一个版本的 GitHub Release 可以包含：

```text
zellij-ai-session-v0.1.0/
├── zellij_ai_session_plugin.wasm
├── zellij-ai-session-index-linux-x86_64
├── zellij-ai-session-index-linux-aarch64
├── zellij-ai-session-index-macos-x86_64
├── zellij-ai-session-index-macos-aarch64
├── SHA256SUMS
└── install.sh
```

其中：

- `.wasm` 插件通常是一份通用文件；
- indexer 根据操作系统和 CPU 架构分别提供；
- `SHA256SUMS` 用于校验下载完整性；
- `install.sh` 负责识别平台、下载和配置。

## 四、一键安装流程

普通用户最终只需要执行：

```bash
curl -fsSL https://raw.githubusercontent.com/snail-vs/zellij-ai-session/main/install.sh | bash
```

安装脚本的完整流程如下：

```text
检测 OS / CPU 架构
        ↓
选择对应的 indexer 下载地址
        ↓
下载 WASM 插件和 native indexer
        ↓
校验 SHA256
        ↓
安装到用户目录
        ↓
写入 Zellij 配置
        ↓
启用 Alt+s 快捷键
        ↓
提示用户重启或重新加载 Zellij
```

推荐的默认安装位置：

```text
~/.local/bin/zellij-ai-session-index
~/.local/share/zellij-ai-session/zellij_ai_session_plugin.wasm
~/.config/zellij/config.kdl
```

不需要 root 权限，也不依赖系统级安装目录。

## 五、自动生成的 Zellij 配置

安装脚本会在 `config.kdl` 中加入托管区块：

```kdl
// zellij-ai-session:begin
shared {
    bind "Alt s" {
        LaunchOrFocusPlugin "file:/path/to/zellij_ai_session_plugin.wasm" {
            floating true
            move_to_focused_tab true
            skip_plugin_cache true
            indexer "/path/to/zellij-ai-session-index"
            open_mode "tab"
        }
    }
}
// zellij-ai-session:end
```

配置含义：

- `shared`：让快捷键适用于不同的 Zellij 输入模式；
- `LaunchOrFocusPlugin`：首次打开插件，后续聚焦已有插件；
- `floating true`：以浮动窗口打开；
- `move_to_focused_tab true`：插件跟随当前 tab；
- `skip_plugin_cache true`：升级后使用最新插件；
- `indexer`：指定 native indexer 的绝对路径；
- `open_mode`：控制历史 Session 恢复到 tab 还是 pane。

## 六、配置更新和卸载

安装脚本使用以下标记保证配置修改是幂等的：

```text
// zellij-ai-session:begin
// zellij-ai-session:end
```

重复执行安装脚本时，会先删除旧的托管区块，再写入最新版本，不会重复追加快捷键。

首次修改已有配置前，会创建备份：

```text
~/.config/zellij/config.kdl.bak.zellij-ai-session
```

卸载时只删除：

- 安装的 indexer；
- 安装的 WASM 插件；
- zellij-ai-session 自己的配置区块。

用户其他 Zellij 配置和备份不会删除。

## 七、跨平台注意事项

| 文件 | 跨平台情况 |
| --- | --- |
| WASM 插件 | 通常可跨平台复用 |
| Linux indexer | 只能用于 Linux |
| macOS indexer | 只能用于 macOS |
| x86_64 indexer | 不能直接用于 ARM |
| aarch64 indexer | 不能直接用于 x86_64 |

发布时还需要考虑：

- Zellij Plugin API 版本兼容；
- Linux glibc 版本兼容；
- 是否额外提供 Linux musl 静态版本；
- macOS Intel 和 Apple Silicon；
- 下载文件的 SHA256 校验；
- GitHub Actions 自动构建和发布。

## 八、源码安装与 Release 安装的关系

源码安装仍然保留，主要用于开发和测试：

```bash
./install.sh --from-source
```

它会在本地构建 indexer 和 WASM 插件。

面向普通用户的 Release 安装执行：

```text
检测平台 → 下载预编译产物 → 校验 → 安装 → 修改 Zellij 配置
```

也可以通过参数安装指定版本：

```bash
./install.sh --version v0.1.0
```

因此最终可以同时支持：

- 开发者：从源码构建；
- 普通用户：下载 Release 一键安装。

## 九、推荐的后续实现

仓库现在已经提供两个 GitHub Actions：

- `ci.yml`：在 push、Pull Request 或手动触发时执行格式、测试和构建检查；
- `release.yml`：检测到 `v*.*.*` tag push 后自动构建并创建 GitHub Release。

创建版本时只需要：

```bash
git tag -a v0.1.0 -m "Release v0.1.0"
git push origin main
git push origin v0.1.0
```

Release workflow 会自动：

1. 验证 tag 对应代码；
2. 构建 Linux x86_64 和 ARM64 indexer；
3. 构建 macOS x86_64 和 Apple Silicon indexer；
4. 构建通用 WASM 插件；
5. 生成 `SHA256SUMS`；
6. 使用 tag 名创建 GitHub Release 并上传产物。

远程 `install.sh` 会根据系统和 CPU 架构下载对应的 Release 产物。

当前 Release workflow 的具体构建步骤是：

1. 在 tag 创建时触发构建；
2. 构建 WASM 插件；
3. 构建多个系统和架构的 indexer；
4. 计算并上传 SHA256；
5. 创建 GitHub Release；
6. 由远程安装脚本下载对应产物。

最终用户体验为：

```bash
curl -fsSL https://.../install.sh | bash
```

用户不需要了解 Rust、WASI、Cargo 或 Zellij 插件内部结构。
