#!/usr/bin/env python3
"""Build the curated snapshot and preview payloads for the demo recording.

The demo shows the navigator with a stable, hand-picked set of projects and
agents, so the recording does not change when the machine gains new sessions.
"""
import hashlib
import json
import os
import time

HERE = os.path.dirname(os.path.abspath(__file__))
NOW = int(time.time() * 1000)
MINUTE = 60 * 1000
HOUR = 60 * MINUTE
DAY = 24 * HOUR


def project_id(directory):
    return hashlib.sha1(directory.encode("utf-8")).hexdigest()[:12]


PROJECTS = [
    "/data/code/typing",
    "/data/code/gitops",
    "/data/code/zellij-ai-session",
    "/data/code/personal-ai-library",
    "/data/code/quicknotes",
    "/data/code/agents/english-coach",
]

# (project dir, agent, title, updated offset, parent index or None, running)
SESSIONS = [
    ("/data/code/typing", "codex", "打通钢琴教学闭环与进度迁移", 4 * MINUTE, None, True),
    ("/data/code/typing", "omp", "修复登录闪烁与异步状态竞争", 26 * MINUTE, None, False),
    ("/data/code/typing", "codex", "补齐盲打关卡的星星结算", 3 * HOUR, 0, False),
    ("/data/code/typing", "opencode", "调整 Ingress 适配 Caddy 和 Nginx", 5 * HOUR, None, False),
    ("/data/code/typing", "claude", "整理音乐知识探索框架的目录", 2 * DAY, None, False),
    ("/data/code/typing", "codex", "统一乐理 API 的错误码", 4 * DAY, None, False),

    ("/data/code/gitops", "opencode", "DeepTutor 内网 Kubernetes 部署方案研究", 12 * MINUTE, None, False),
    ("/data/code/gitops", "codex", "Prometheus 采集 LXC 容器指标", 40 * MINUTE, None, False),
    ("/data/code/gitops", "omp", "修复 Grafana 容器表格布局", 3 * HOUR, None, False),
    ("/data/code/gitops", "codex", "OpenMAIC 接入 ChatGPT Codex OAuth", 8 * HOUR, 6, False),
    ("/data/code/gitops", "opencode", "把镜像推送到 Gitea 和 GitHub", 1 * DAY, 6, False),

    ("/data/code/zellij-ai-session", "codex", "兼容 OpenCode 1.x 与 2.0 的会话存储", 2 * MINUTE, None, True),
    ("/data/code/zellij-ai-session", "codex", "新增 OMP 会话支持", 30 * MINUTE, 11, False),
    ("/data/code/zellij-ai-session", "omp", "调研 oh-my-pi 的会话目录结构", 55 * MINUTE, None, False),
    ("/data/code/zellij-ai-session", "codex", "修复插件在 wasm 目标下的编译", 3 * HOUR, None, False),

    ("/data/code/personal-ai-library", "codex", "新增缓存与查词页面", 90 * MINUTE, None, False),
    ("/data/code/personal-ai-library", "omp", "校对生词本的收录约束", 6 * HOUR, None, False),
    ("/data/code/personal-ai-library", "codex", "补充 DOCX 来源优先级", 1 * DAY, None, False),

    ("/data/code/quicknotes", "codex", "清理上传完成记录并优化续传", 2 * HOUR, None, False),
    ("/data/code/quicknotes", "opencode", "把登录状态写入 Redis", 1 * DAY, None, False),

    ("/data/code/agents/english-coach", "omp", "生成教学资料索引", 4 * HOUR, None, False),
    ("/data/code/agents/english-coach", "codex", "校验英语学习账号权限开关", 20 * HOUR, None, False),
]

# Transcripts shown in the details pane, keyed by the session title.
PREVIEWS_BY_TITLE = {
    "打通钢琴教学闭环与进度迁移": [
        {"role": "user", "text": "钢琴教学闭环现在卡在进度迁移上，帮我看看还差哪些环节。"},
        {"role": "assistant", "text": "先梳理迁移涉及的三张表，再确认回放逻辑，最后补上星星结算的幂等判断。"},
        {"role": "user", "text": "星星结算重复发过一次，帮忙查一下。"},
        {"role": "assistant", "text": "结算接口缺少幂等键，已按 sessionId + 关卡号补齐，重复请求现在直接返回已有记录。"},
    ],
    "修复登录闪烁与异步状态竞争": [
        {"role": "user", "text": "登录页偶尔闪烁，控制台有重复的状态更新。"},
        {"role": "assistant", "text": "定位到竞态：登出后未取消的请求仍会写回状态。已改为请求带会话版本号，过期响应直接丢弃。"},
    ],
    "兼容 OpenCode 1.x 与 2.0 的会话存储": [
        {"role": "user", "text": "OpenCode 升到 2.0 之后会话列表空了，帮我看看存储结构变了什么。"},
        {"role": "assistant", "text": "2.0 把新会话写进 session_v2 和 session_message，两张旧表只留历史。索引器改为优先读新表，旧表作为回退即可同时兼容 1.x 和 2.x。"},
    ],
    "新增 OMP 会话支持": [
        {"role": "user", "text": "oh-my-pi 也能加进来吗？它和 pi 的会话目录长得一样。"},
        {"role": "assistant", "text": "格式是同一套，但 OMP 把自动标题写在首行、session 条目挪到第二行。把解析抽成共用模块后两边都能读。"},
        {"role": "user", "text": "那按 uuid 恢复还是按路径？"},
        {"role": "assistant", "text": "OMP 用 uuid：omp --resume <id>。Pi 仍然按路径，因为它只认 --session <path>。"},
    ],
    "调研 oh-my-pi 的会话目录结构": [
        {"role": "user", "text": "帮我看看 omp 的会话都存在哪，怎么按项目分组。"},
        {"role": "assistant", "text": "在 ~/.omp/agent/sessions/<项目>/<时间戳>_<uuid>.jsonl，一个项目一个目录。子 agent 的转录单独放在会话目录里，扫描时只取一层。"},
    ],
    "统一乐理 API 的错误码": [
        {"role": "user", "text": "统一一下乐理 API 的错误码。"},
        {"role": "assistant", "text": "已按资源类型归并为 400/403/404/409/422 五类，客户端改动集中在映射表一处。"},
    ],
    "DeepTutor 内网 Kubernetes 部署方案研究": [
        {"role": "user", "text": "内网 Kubernetes 部署方案调研，输出可以直接落地的文档。"},
        {"role": "assistant", "text": "已整理为镜像源、探针、Ingress 与存储四部分，每部分都给出可直接套用的清单。"},
    ],
}


def build():
    projects = []
    sessions = []
    ids = []
    for directory in PROJECTS:
        pid = project_id(directory)
        projects.append(
            {
                "project": {
                    "id": pid,
                    "name": os.path.basename(directory),
                    "root_directory": directory,
                    "origin": "auto",
                },
                "session_count": 0,
                "running_count": 0,
                "latest_updated_at_ms": None,
            }
        )

    for index, (directory, agent, title, offset, parent, running) in enumerate(SESSIONS):
        pid = project_id(directory)
        native = f"{agent}-{index}"
        session_id = f"{agent}:{native}"
        ids.append(session_id)
        parent_id = ids[parent] if parent is not None else None
        updated = NOW - offset
        sessions.append(
            {
                "id": session_id,
                "agent": agent,
                "title": title,
                "project_id": pid,
                "directory": directory,
                "created_at_ms": updated - 3 * HOUR,
                "updated_at_ms": updated,
                "agent_session_id": native,
                "status": "running" if running else "historical",
                "runtime": None,
                "parent_id": parent_id,
                "native_available": True,
                "selected_cwd": None,
                "work_state": None,
            }
        )
        summary = next(p for p in projects if p["project"]["id"] == pid)
        summary["session_count"] += 1
        summary["running_count"] += 1 if running else 0
        if summary["latest_updated_at_ms"] is None or updated > summary["latest_updated_at_ms"]:
            summary["latest_updated_at_ms"] = updated

    for summary in projects:
        if summary["latest_updated_at_ms"] is None:
            summary["latest_updated_at_ms"] = NOW - DAY

    snapshot = {
        "version": 1,
        "generated_at_ms": NOW,
        "projects": projects,
        "sessions": sessions,
        "warnings": [],
    }
    with open(os.path.join(HERE, "snapshot.json"), "w", encoding="utf-8") as handle:
        json.dump(snapshot, handle, ensure_ascii=False, indent=2)

    by_title = {session[2]: index for index, session in enumerate(SESSIONS)}
    previews = {}
    for title, messages in PREVIEWS_BY_TITLE.items():
        index = by_title[title]
        native = f"{SESSIONS[index][1]}-{index}"
        previews[native] = {
            "session_id": native,
            "messages": messages,
            "note": None,
        }
    with open(os.path.join(HERE, "previews.json"), "w", encoding="utf-8") as handle:
        json.dump(previews, handle, ensure_ascii=False, indent=2)
    print(f"{len(sessions)} sessions across {len(projects)} projects")


if __name__ == "__main__":
    build()
