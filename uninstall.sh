#!/usr/bin/env bash
set -euo pipefail

BIN_DIR="${ZELLIJ_AI_SESSION_BIN_DIR:-${XDG_BIN_HOME:-$HOME/.local/bin}}"
DATA_DIR="${ZELLIJ_AI_SESSION_DATA_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/zellij-ai-session}"
CONFIG_FILE="${ZELLIJ_AI_SESSION_CONFIG_FILE:-${ZELLIJ_CONFIG_FILE:-${XDG_CONFIG_HOME:-$HOME/.config}/zellij/config.kdl}}"

command -v python3 >/dev/null || {
    echo "python3 is required to update Zellij config" >&2
    exit 1
}

export ZELLIAI_CONFIG_FILE="$CONFIG_FILE"
python3 <<'PY'
import os
import re
import stat
import tempfile
from pathlib import Path

path = Path(os.environ["ZELLIAI_CONFIG_FILE"])
begin = "// zellij-ai-session:begin"
end = "// zellij-ai-session:end"
if path.exists():
    content = path.read_text()
    pattern = re.compile(rf"(?ms)^\s*{re.escape(begin)}\n.*?^\s*{re.escape(end)}\s*\n?")
    updated = pattern.sub("", content)
    if updated != content:
        mode = stat.S_IMODE(path.stat().st_mode)
        with tempfile.NamedTemporaryFile("w", encoding="utf-8", dir=path.parent, delete=False) as temp:
            temp.write(updated)
            temporary = Path(temp.name)
        os.chmod(temporary, mode)
        os.replace(temporary, path)
        print(f"Removed managed keybind from {path}")
    else:
        print(f"No managed keybind found in {path}")
PY

rm -f "$BIN_DIR/zellij-ai-session-index"
rm -f "$DATA_DIR/zellij_ai_session_plugin.wasm"
rmdir "$DATA_DIR" 2>/dev/null || true

echo "Uninstalled zellij-ai-session binaries."
echo "Any config backup remains at: $CONFIG_FILE.bak.zellij-ai-session"
