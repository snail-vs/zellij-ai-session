#!/usr/bin/env bash
set -euo pipefail

OPEN_MODE=tab
KEY="Alt s"
INSTALL_KEYBIND=1

usage() {
    cat <<'EOF'
Usage: ./install.sh [--open-mode tab|pane] [--key "Alt s"] [--no-keybind]

Builds the indexer and WASI plugin, installs them for the current user, and
adds an idempotent Alt+s launcher to the Zellij config.
EOF
}

while (($#)); do
    case "$1" in
        --open-mode)
            [[ $# -ge 2 ]] || { echo "--open-mode requires tab or pane" >&2; exit 2; }
            OPEN_MODE=$2
            shift 2
            ;;
        --key)
            [[ $# -ge 2 ]] || { echo "--key requires a Zellij key" >&2; exit 2; }
            KEY=$2
            shift 2
            ;;
        --no-keybind) INSTALL_KEYBIND=0; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
    esac
done

case "$OPEN_MODE" in
    tab|pane) ;;
    *) echo "open mode must be tab or pane" >&2; exit 2 ;;
esac

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
BIN_DIR="${ZELLIJ_AI_SESSION_BIN_DIR:-${XDG_BIN_HOME:-$HOME/.local/bin}}"
DATA_DIR="${ZELLIJ_AI_SESSION_DATA_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/zellij-ai-session}"
CONFIG_FILE="${ZELLIJ_AI_SESSION_CONFIG_FILE:-${ZELLIJ_CONFIG_FILE:-${XDG_CONFIG_HOME:-$HOME/.config}/zellij/config.kdl}}"
INDEXER_PATH="$BIN_DIR/zellij-ai-session-index"
PLUGIN_PATH="$DATA_DIR/zellij_ai_session_plugin.wasm"

command -v cargo >/dev/null || { echo "cargo is required (install Rust via rustup.rs)" >&2; exit 1; }
command -v rustup >/dev/null || { echo "rustup is required to install wasm32-wasip1" >&2; exit 1; }
command -v python3 >/dev/null || { echo "python3 is required to update Zellij config" >&2; exit 1; }

if ! rustup target list --installed | grep -qx wasm32-wasip1; then
    echo "Installing Rust target wasm32-wasip1..."
    rustup target add wasm32-wasip1
fi

echo "Building indexer..."
cargo build --manifest-path "$ROOT_DIR/Cargo.toml" -p zellij-ai-session-index --release
echo "Building Zellij plugin..."
cargo build --manifest-path "$ROOT_DIR/Cargo.toml" \
    -p zellij-ai-session-plugin --target wasm32-wasip1 --features wasm --release

mkdir -p "$BIN_DIR" "$DATA_DIR"
cp "$ROOT_DIR/target/release/zellij-ai-session-index" "$INDEXER_PATH"
cp "$ROOT_DIR/target/wasm32-wasip1/release/zellij_ai_session_plugin.wasm" "$PLUGIN_PATH"
chmod 755 "$INDEXER_PATH"
chmod 644 "$PLUGIN_PATH"

if ((INSTALL_KEYBIND)); then
    export ZELLIAI_CONFIG_FILE="$CONFIG_FILE"
    export ZELLIAI_PLUGIN_PATH="$PLUGIN_PATH"
    export ZELLIAI_INDEXER_PATH="$INDEXER_PATH"
    export ZELLIAI_OPEN_MODE="$OPEN_MODE"
    export ZELLIAI_KEY="$KEY"
    python3 <<'PY'
import json
import os
import re
import stat
import tempfile
from pathlib import Path

config = Path(os.environ["ZELLIAI_CONFIG_FILE"])
plugin = Path(os.environ["ZELLIAI_PLUGIN_PATH"])
indexer = Path(os.environ["ZELLIAI_INDEXER_PATH"])
mode = os.environ["ZELLIAI_OPEN_MODE"]
key = os.environ["ZELLIAI_KEY"]
begin = "// zellij-ai-session:begin"
end = "// zellij-ai-session:end"


def close_brace(text, opening):
    depth = 0
    quote = False
    line_comment = False
    block_comment = False
    i = opening
    while i < len(text):
        c = text[i]
        n = text[i + 1] if i + 1 < len(text) else ""
        if line_comment:
            if c == "\n":
                line_comment = False
        elif block_comment:
            if c == "*" and n == "/":
                block_comment = False
                i += 1
        elif quote:
            if c == "\\":
                i += 1
            elif c == '"':
                quote = False
        elif c == "/" and n == "/":
            line_comment = True
            i += 1
        elif c == "/" and n == "*":
            block_comment = True
            i += 1
        elif c == '"':
            quote = True
        elif c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return i
        i += 1
    raise SystemExit("Could not parse the Zellij KDL config")


def remove_managed(text):
    pattern = re.compile(rf"(?ms)^\s*{re.escape(begin)}\n.*?^\s*{re.escape(end)}\s*\n?")
    return pattern.sub("", text)


def remove_old_plugin_binding(text):
    for binding_key in dict.fromkeys((key, "Alt a")):
        needle = f'bind {json.dumps(binding_key)}'
        cursor = 0
        output = []
        while True:
            start = text.find(needle, cursor)
            if start < 0:
                output.append(text[cursor:])
                text = "".join(output)
                break
            opening = text.find("{", start + len(needle))
            if opening < 0:
                output.append(text[cursor:])
                text = "".join(output)
                break
            closing = close_brace(text, opening)
            body = text[start:closing + 1]
            if "zellij_ai_session_plugin.wasm" not in body:
                output.append(text[cursor:closing + 1])
            else:
                output.append(text[cursor:start])
            cursor = closing + 1
    return text


def keybinds_block(text):
    match = re.search(r"(?m)^\s*keybinds\b", text)
    if not match:
        return None
    opening = text.find("{", match.end())
    if opening < 0:
        raise SystemExit("Found keybinds without a block")
    return opening, close_brace(text, opening)


if config.exists():
    original = config.read_text()
    file_mode = stat.S_IMODE(config.stat().st_mode)
else:
    original = ""
    file_mode = 0o600

content = remove_managed(original)
content = remove_old_plugin_binding(content)
needle = f'bind {json.dumps(key)}'
if needle in content:
    raise SystemExit(f"Zellij key '{key}' is already used; rerun with --key 'Alt z' or edit the config")

managed = f'''    {begin}
    shared {{
        bind {json.dumps(key)} {{
            LaunchOrFocusPlugin {json.dumps("file:" + str(plugin))} {{
                floating true
                move_to_focused_tab true
                skip_plugin_cache true
                indexer {json.dumps(str(indexer))}
                open_mode {json.dumps(mode)}
            }}
        }}
    }}
    {end}
'''

block = keybinds_block(content)
if block is None:
    content = content.rstrip() + "\n\nkeybinds {\n" + managed + "}\n"
else:
    _, closing = block
    content = content[:closing] + "\n" + managed + content[closing:]

config.parent.mkdir(parents=True, exist_ok=True)
if config.exists():
    backup = config.with_name(config.name + ".bak.zellij-ai-session")
    if not backup.exists():
        backup.write_text(original)
with tempfile.NamedTemporaryFile("w", encoding="utf-8", dir=config.parent, delete=False) as temp:
    temp.write(content)
    temporary = Path(temp.name)
os.chmod(temporary, file_mode)
os.replace(temporary, config)
PY
fi

echo
echo "Installed zellij-ai-session"
echo "  indexer: $INDEXER_PATH"
echo "  plugin:  $PLUGIN_PATH"
if ((INSTALL_KEYBIND)); then
    echo "  key:     $KEY"
    echo "  config:  $CONFIG_FILE"
    echo "Restart Zellij or reload its config, then press $KEY."
else
    echo "Keybind installation skipped (--no-keybind)."
fi
