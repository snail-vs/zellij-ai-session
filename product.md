# zellij-ai-session 产品方案

## 1. 产品定位

`zellij-ai-session` 是运行在 Zellij 中的统一 AI Coding Session Navigator。

它解决的核心问题不是“管理 Zellij session”，而是：

> 在一个统一 TUI 中，以项目为中心浏览、定位、恢复和创建 Codex、OpenCode、Pi、Reasonix 等 AI Coding Agent 的会话。

当前不同 Agent 都有自己的 Session：

```text
Codex
  └── sessions

OpenCode
  └── sessions

Pi
  └── sessions

Reasonix
  └── sessions
```

用户必须：

```text
cd project-a
codex resume

cd project-b
opencode ...

cd project-c
pi ...
```

`zellij-ai-session` 将它们统一成：

```text
Projects
├── opencode
│   ├── Codex      session A
│   ├── OpenCode   session B
│   └── Codex      session C
│
├── personal-ai-library
│   ├── Codex      session D
│   └── Pi         session E
│
└── k8s-lab
    ├── OpenCode   session F
    └── Reasonix   session G
```

产品的核心抽象是：

```text
Project
   ↓
AI Session
   ↓
Agent
   ↓
Runtime（可选 Zellij pane）
```

而不是：

```text
Agent
   ↓
Project
   ↓
Session
```

---

# 2. 为什么一级必须是 Project / Directory

## 2.1 用户工作的对象是 Project

实际工作思维通常是：

```text
今天继续搞 opencode
今天继续搞 k8s-lab
今天继续做 personal-ai-library
```

而不是：

```text
今天我要使用 Codex
今天我要使用 OpenCode
```

Agent 是工具，Project 才是工作对象。

因此：

```text
Project = 稳定上下文
Agent   = 可替换执行器
Session = 某次工作过程
```

这是产品最重要的模型选择。

## 2.2 同一个项目可能混用多个 Agent

例如：

```text
personal-ai-library
├── Codex
│   └── 实现 reader parser
│
├── OpenCode
│   └── 调查 TTS bug
│
└── Pi
    └── 重构数据库
```

如果一级按照 Agent：

```text
Codex
├── personal-ai-library
├── opencode
└── k8s-lab

OpenCode
├── personal-ai-library
└── k8s-lab
```

同一个项目被拆散了。

而按照 Project：

```text
personal-ai-library
├── Codex
├── OpenCode
└── Pi
```

所有工作上下文天然聚合。

## 2.3 Directory 和 Project 的关系

第一版可以直接认为：

```text
Project ≈ Directory
```

例如：

```text
/home/user/code/opencode
/home/user/code/personal-ai-library
/home/user/lab/k8s
```

分别就是三个 Project。

但是数据模型里不要直接把：

```text
project_id = directory
```

写死。

应该抽象：

```text
Project {
    id
    name
    root_directory
}
```

第一版：

```text
id             = normalized directory
name           = basename(directory)
root_directory = directory
```

以后可以支持：

```text
Git worktree
Monorepo
自定义 Project
多个目录归属于同一个 Workspace
```

所以 UI 可以叫：

# Projects

而不是：

# Directories

底层第一版仍然通过 directory 自动发现 Project。

---

# 3. 主界面信息架构

建议第一版不要直接做复杂 Tree，而采用两级导航。

## Level 1：Projects

```text
AI Sessions

> personal-ai-library       8 sessions   2 running
  opencode                  5 sessions   1 running
  k8s-lab                   4 sessions
  zellij-ai-session         3 sessions   1 running
```

Project 排序：

```text
1. 有 running / needs-input 的项目
2. 最近有 Session 更新的项目
3. 其他项目按最近更新时间
```

Project 行展示：

```text
{name}     {session_count}    {runtime_status}
```

可选 footer：

```text
~/code/personal-ai-library
```

按 Enter：

```text
Projects
   ↓
Project Sessions
```

---

# 4. Project 内部 Session 页面

进入：

```text
personal-ai-library
```

展示：

```text
personal-ai-library

● Codex      实现高频词块              12:03
● OpenCode   修复 reader parser        11:24
○ Codex      设计数据库 schema         Yesterday
○ Pi         调查 TTS                  Aug 12
○ Codex      PDF parsing               Aug 08
```

其中：

```text
● = 当前存在 Zellij runtime
○ = 只有历史 Session
```

后面可以增加：

```text
◉ needs input
● working
◐ idle
○ historical
```

但 MVP 不需要做到这么复杂。

每条 Session 的数据：

```text
title
agent
updated_at
runtime status
```

目录已经属于当前 Project，因此不需要重复展示。

---

# 5. Agent 的定位

Agent 不作为一级导航。

Agent 应该承担三个角色。

## 角色一：Session 属性

```text
Session {
    agent: codex
}
```

UI：

```text
Codex    重构 Session Navigator
```

## 角色二：Filter

按 `a`：

```text
Agent Filter

✓ All
  Codex
  OpenCode
  Pi
  Reasonix
```

于是可以快速变成：

```text
personal-ai-library

Codex only

  实现高频词块
  数据库 schema
  reader parser
```

## 角色三：创建 Session 时选择 Agent

Project 页面按：

```text
n
```

弹出：

```text
New AI Session

> Codex
  OpenCode
  Pi
  Reasonix
```

然后自动：

```text
cwd = project.root_directory
↓
创建 Zellij pane/tab
↓
启动对应 Agent
```

所以：

> Project 是导航维度，Agent 是执行维度。

---

# 6. 全局 Agent View

虽然 Agent 不应该做默认一级入口，但可以提供第二种 View。

例如按：

```text
v
```

切换：

```text
Project View
Agent View
```

Agent View：

```text
Agents

Codex                    12
├── personal-ai-library   4
├── opencode              5
└── k8s-lab               3

OpenCode                  7
├── personal-ai-library   2
└── k8s-lab               5

Pi                        3
└── personal-ai-library   3
```

这个页面主要用于回答：

```text
我有哪些 Codex Session？
现在运行了几个 OpenCode？
Reasonix 最近用在哪些项目？
```

所以最终模型不是二选一：

```text
Project OR Agent
```

而是：

```text
默认主视图：Project → Session
辅助视图：Agent → Project → Session
```

但 MVP 只实现 Project View。

---

# 7. Session 状态模型

需要明确区分：

```text
AI Session
```

和：

```text
Zellij Runtime
```

一个 AI Session 可以存在，但没有运行。

例如：

```text
Codex session abc123

AI session:
存在

Zellij pane:
不存在
```

状态：

```text
historical
```

如果：

```text
Codex session abc123
      │
      └── Zellij session dev
           └── tab 2
                └── pane 5
```

状态：

```text
running
```

统一结构：

```text
AISession {
    id
    agent
    title

    project_id
    directory

    created_at
    updated_at

    agent_session_id

    runtime?: {
        zellij_session
        tab_id
        pane_id
    }
}
```

第一版只需要：

```text
running
historical
```

之后再增加：

```text
working
idle
needs_input
error
```

---

# 8. Enter 的核心行为

这是整个产品最关键的体验。

用户不应该思考：

```text
这个 Session 是否已经运行？
resume 命令是什么？
在哪个 Zellij pane？
```

只需要：

```text
Enter
```

## Session 正在运行

```text
Enter
 ↓
找到 runtime
 ↓
switch_session_with_focus()
 ↓
跳到现有 pane
```

## Session 没有运行

```text
Enter
 ↓
adapter.resumeCommand()
 ↓
创建 Zellij pane
 ↓
cwd = project.directory
 ↓
运行：

codex resume xxx
```

然后：

```text
historical
↓
running
```

所以 UI 行为统一：

> Enter = 去这个 AI Session。

这应该成为整个产品最核心的交互原则。

---

# 9. Agent Adapter

所有 Agent 的差异封装到 Adapter。

统一接口概念：

```go
type AgentAdapter interface {
    Name() string

    DetectProcess(command string) bool

    ListSessions() ([]Session, error)

    ResumeCommand(session Session) []string

    NewCommand(project Project) []string
}
```

Codex：

```text
CodexAdapter
├── 找 Codex session store
├── 解析 session ID
├── directory
├── title
├── updated_at
└── codex resume ...
```

OpenCode：

```text
OpenCodeAdapter
├── 查询 OpenCode DB/API
├── projectID
├── directory
├── title
└── resume command
```

Pi：

```text
PiAdapter
```

Reasonix：

```text
ReasonixAdapter
```

未来：

```text
Claude Code
Gemini CLI
Cursor Agent
Aider
```

UI 不需要发生变化。

---

# 10. 技术架构

不建议 fork Zellij。

采用：

```text
zellij-ai-session
│
├── Zellij Plugin
│
└── Native Indexer
```

详细：

```text
┌──────────────────────────────────────┐
│ Zellij                               │
│                                      │
│   zellij-ai-session.wasm             │
│                                      │
│   Project List                       │
│   Session List                       │
│   Search / Filter                    │
│   Keyboard Navigation                │
│   Jump / Resume                      │
└──────────────────┬───────────────────┘
                   │
                   │ pipe / command
                   ▼
┌──────────────────────────────────────┐
│ zellij-ai-session-index              │
│                                      │
│ Project Discovery                    │
│ Session Index                        │
│ Runtime Detection                    │
│                                      │
│ adapters/                            │
│   codex                              │
│   opencode                           │
│   pi                                 │
│   reasonix                           │
└───────────────┬──────────────────────┘
                │
        ┌───────┼─────────┐
        ▼       ▼         ▼
      Codex  OpenCode     Pi
      state    DB        state
```

原因：

Zellij Plugin 负责：

```text
UI
快捷键
Zellij pane/session 操作
```

Native helper 负责：

```text
文件系统
SQLite
Agent 数据解析
索引
复杂逻辑
```

避免把所有东西塞进 WASM。

---

# 11. Project Discovery

Project 不应该要求用户先配置。

第一版直接根据 Agent Session 自动发现：

```text
Codex session
   cwd=/home/rongfei/code/opencode

OpenCode session
   directory=/home/rongfei/code/opencode

Pi session
   cwd=/home/rongfei/code/opencode
```

归并：

```text
/home/rongfei/code/opencode
             ↓
        Project(opencode)
```

也就是说：

```text
Sessions
   ↓
提取 cwd
   ↓
normalize directory
   ↓
group by project
```

因此用户第一次启动：

```bash
zellij
```

打开插件就已经看到：

```text
Projects

opencode
personal-ai-library
k8s
...
```

不需要创建 Project。

以后再增加：

```toml
[projects]
roots = [
  "~/code",
  "~/work",
  "~/lab"
]
```

用于主动扫描没有 AI session 的 Project。

---

# 12. 搜索

这是第二重要能力。

按：

```text
/
```

不局限于当前 Project。

进入：

```text
Search All Sessions
```

输入：

```text
volcano
```

结果：

```text
k8s-lab
  OpenCode   volcano scheduler
  Codex      investigate gang scheduling

personal-ai-library
  Codex      write volcano notes
```

搜索字段：

```text
project
directory
agent
session title
first prompt（后续）
```

这可以解决：

> 记得以前讨论过这个问题，但忘了在哪个目录、哪个 Agent。

这是统一 Session Manager 相比各 Agent 自己 resume 最大的价值之一。

---

# 13. MVP

第一版必须严格控制范围。

## MVP 只完成

### 1. Project discovery

通过 Agent Session 的 cwd/directory 聚合项目。

### 2. Session index

首批支持：

```text
Codex
OpenCode
```

先不要四个一起上。

### 3. 两级导航

```text
Projects
   ↓
Sessions
```

### 4. Runtime detection

识别当前 Zellij panes 中：

```text
codex
opencode
```

并关联历史 Session。

### 5. Enter

```text
running → jump

historical → resume
```

### 6. Search

全局按 session title 搜索。

---

# 14. MVP 明确不做

第一阶段不做：

```text
AI working / idle / needs-input 状态判断
跨机器 Session
Tailscale
tmux 支持
聊天内容预览
Session rename
Session delete
Session migration
多 Agent 同时启动
任务编排
Agent 自动选择
Token / cost
Git branch 管理
```

这些都会迅速把项目变成 ccmux。

`zellij-ai-session` 第一阶段只做：

> Find → Navigate → Resume。

---

# 15. 第一版界面

启动：

```text
┌─ AI Sessions ──────────────────────────────────┐
│                                                │
│ Projects                                       │
│                                                │
│ > personal-ai-library          8   ●2          │
│   opencode                     5   ●1          │
│   k8s-lab                      4               │
│   zellij-ai-session            3   ●1          │
│                                                │
│                                                │
│ / search   a agents   n new   q close          │
└────────────────────────────────────────────────┘
```

Enter：

```text
┌─ personal-ai-library ──────────────────────────┐
│                                                │
│ > ● Codex     实现高频词块             12:03   │
│   ● OpenCode  reader parser bug        11:24   │
│   ○ Codex     数据库 schema            Aug 14   │
│   ○ Pi        TTS 调研                 Aug 12   │
│                                                │
│                                                │
│ Enter open   n new   / search   Esc back       │
└────────────────────────────────────────────────┘
```

这已经足够形成一个完整产品。

---

# 16. 仓库结构建议

```text
zellij-ai-session/
├── README.md
├── Cargo.toml
│
├── crates/
│   ├── plugin/
│   │   ├── src/
│   │   │   ├── app.rs
│   │   │   ├── ui/
│   │   │   ├── keymap.rs
│   │   │   └── zellij.rs
│   │   └── Cargo.toml
│   │
│   ├── indexer/
│   │   ├── src/
│   │   │   ├── project.rs
│   │   │   ├── session.rs
│   │   │   ├── runtime.rs
│   │   │   └── adapters/
│   │   │       ├── codex.rs
│   │   │       └── opencode.rs
│   │   └── Cargo.toml
│   │
│   └── core/
│       ├── src/
│       │   ├── project.rs
│       │   ├── session.rs
│       │   └── agent.rs
│       └── Cargo.toml
│
└── examples/
```

第一版甚至可以先把 plugin 和 indexer 放一起，模型稳定以后再拆。

---

# 17. 产品原则

建议从一开始确定五条原则。

**1. Project-first**

```text
Project > Session > Agent
```

不是：

```text
Agent > Session
```

**2. Agent-independent**

UI 不依赖具体 Agent。

新增 Agent = 新增 Adapter。

**3. Multiplexer-native**

充分利用 Zellij，但不修改 Zellij Core。

**4. Zero configuration first**

能够从已有 AI Session 自动发现 Project。

**5. One action to continue**

用户不关心：

```text
jump
attach
resume
create pane
cd directory
```

用户只关心：

```text
Enter → 回到我要继续的工作。
```

---

# 18. 一句话定义

README 首页可以先把产品定义成：

> **zellij-ai-session — Project-first session navigator for AI coding agents in Zellij.**

中文：

> **以项目为中心，在 Zellij 中统一浏览、定位和恢复多个 AI Coding Agent 会话。**

核心结构只有这一张图：

```text
                    Projects
                       │
          ┌────────────┼────────────┐
          ▼            ▼            ▼
       opencode      k8s-lab      personal-ai
          │            │            │
       Sessions      Sessions      Sessions
          │            │            │
    ┌─────┼─────┐      │       ┌────┼────┐
    ▼     ▼     ▼      ▼       ▼    ▼    ▼
 Codex   OC    Pi    Codex    OC   Codex Pi
```

**Project 是用户的工作上下文，Session 是工作的历史轨迹，Agent 只是执行这项工作的工具。**
