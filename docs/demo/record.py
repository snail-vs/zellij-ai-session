#!/usr/bin/env python3
"""Record the demo by driving the real Zellij plugin inside tmux.

Zellij runs with an isolated configuration that loads the freshly built WASI
plugin, and the fake indexer feeds it the curated data. Each step sends keys
to Zellij, waits for the UI to settle and captures the resulting screen, so
the recording shows the plugin exactly as a user would see it.
"""
import os
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", ".."))
WORK = os.path.join(HERE, "work")
FRAMES = os.path.join(WORK, "frames")
SESSION = "zellij-ai-session-demo"
COLS, ROWS = 240, 46
PLUGIN = os.path.join(
    REPO, "target", "wasm32-wasip1", "release", "zellij_ai_session_plugin.wasm"
)

ENV = dict(os.environ)
ENV.update(
    {
        "ZELLIJ_CONFIG_DIR": os.path.join(WORK, "config"),
        "ZELLIJ_SOCKET_DIR": os.path.join(WORK, "socket"),
        "XDG_DATA_HOME": os.path.join(WORK, "xdg-data"),
        "XDG_CACHE_HOME": os.path.join(WORK, "xdg-cache"),
    }
)

CONFIG = """\
show_release_notes false
mouse_hover_tips false

keybinds {{
    shared {{
        bind "Alt s" {{
            LaunchOrFocusPlugin "file:{plugin}" {{
                floating true
                move_to_focused_tab true
                skip_plugin_cache true
                indexer "{indexer}"
                open_mode "tab"
            }}
        }}
    }}
}}
"""

# (keys, settle seconds)
STEPS = [
    ([], 2.0),                    # overview
    (["Down"], 2.5),              # OpenCode 2.0 session
    (["Down"], 2.5),              # OMP support session
    (["Down"], 2.5),              # omp session
    (["Down"] * 3, 3.0),          # typing project session
    (["Down"] * 2, 3.0),          # omp session in typing
    (["Down"] * 5, 3.0),          # opencode session in gitops
    (["/"], 1.5),                 # search view
    ("omp", 2.0),
    (["Escape"], 2.5),            # back to the tree
    ([], 1.5),
]


def tmux(*args, check=True):
    result = subprocess.run(["tmux", *args], capture_output=True, text=True, env=ENV)
    if check and result.returncode != 0:
        raise SystemExit(f"tmux {args} failed: {result.stderr}")
    return result.stdout


def main():
    if not os.path.exists(PLUGIN):
        raise SystemExit(
            f"build the plugin first:\n"
            f"  cargo build -p zellij-ai-session-plugin --target wasm32-wasip1 --features wasm --release"
        )
    if shutil.which("tmux") is None:
        raise SystemExit("tmux is required to record the demo")

    os.makedirs(FRAMES, exist_ok=True)
    for name in os.listdir(FRAMES):
        os.remove(os.path.join(FRAMES, name))
    for name in ("config", "socket", "xdg-data", "xdg-cache"):
        os.makedirs(os.path.join(WORK, name), exist_ok=True)

    # A short plugin path keeps the floating pane title readable.
    local_plugin = os.path.join(WORK, "plugin.wasm")
    shutil.copy(PLUGIN, local_plugin)
    indexer = os.path.join(HERE, "indexer")
    os.chmod(indexer, 0o755)
    with open(os.path.join(WORK, "config", "config.kdl"), "w", encoding="utf-8") as handle:
        handle.write(CONFIG.format(plugin=local_plugin, indexer=indexer))

    tmux("kill-session", "-t", SESSION, check=False)
    time.sleep(1)
    tmux("new-session", "-d", "-s", SESSION, "-x", str(COLS), "-y", str(ROWS), "zellij")
    time.sleep(6)
    # Open the navigator and accept the permission prompt.
    tmux("send-keys", "-t", SESSION, "M-s")
    time.sleep(3)
    tmux("send-keys", "-t", SESSION, "y")
    time.sleep(3)

    for index, (keys, settle) in enumerate(STEPS):
        if keys:
            tmux("send-keys", "-t", SESSION, *keys)
        time.sleep(settle)
        path = os.path.join(FRAMES, f"{index:02d}.txt")
        with open(path, "w", encoding="utf-8") as handle:
            handle.write(tmux("capture-pane", "-p", "-e", "-t", SESSION))
        print(f"  frame {index:02d}: {keys or ['(wait)']}")

    tmux("kill-session", "-t", SESSION, check=False)
    print(f"captured {len(STEPS)} frames into {FRAMES}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
