#!/usr/bin/env bash
set -euo pipefail

OPEN_MODE=tab
KEY="Alt s"
INSTALL_KEYBIND=1
INSTALL_MODE=auto
RELEASE_VERSION="${ZELLIJ_AI_SESSION_VERSION:-latest}"
REPOSITORY="${ZELLIJ_AI_SESSION_REPO:-snail-vs/zellij-ai-session}"
RELEASE_BASE_URL="${ZELLIJ_AI_SESSION_RELEASE_BASE_URL:-}"

usage() {
    cat <<'EOF'
Usage: ./install.sh [options]

Install zellij-ai-session for the current user.

Options:
  --open-mode MODE  Restore sessions in tab or pane (default: tab)
  --key KEY         Zellij key binding (default: Alt s)
  --version VERSION Release tag to install (default: latest)
  --repo OWNER/REPO GitHub repository (default: snail-vs/zellij-ai-session)
  --from-source     Build from the local source checkout
  --download        Download a prebuilt GitHub Release
  --no-keybind      Install files without editing Zellij config
  -h, --help        Show this help

When run inside the source checkout, local builds are used automatically.
When run through curl or from another directory, the latest GitHub Release is
downloaded automatically.
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
        --version)
            [[ $# -ge 2 ]] || { echo "--version requires a release tag" >&2; exit 2; }
            RELEASE_VERSION=$2
            shift 2
            ;;
        --repo)
            [[ $# -ge 2 ]] || { echo "--repo requires OWNER/REPO" >&2; exit 2; }
            REPOSITORY=$2
            shift 2
            ;;
        --from-source) INSTALL_MODE=source; shift ;;
        --download) INSTALL_MODE=download; shift ;;
        --no-keybind) INSTALL_KEYBIND=0; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
    esac
done

case "$OPEN_MODE" in
    tab|pane) ;;
    *) echo "open mode must be tab or pane" >&2; exit 2 ;;
esac

ROOT_DIR="$(cd -- "$(dirname -- "$0")" && pwd)"
BIN_DIR="${ZELLIJ_AI_SESSION_BIN_DIR:-${XDG_BIN_HOME:-$HOME/.local/bin}}"
DATA_DIR="${ZELLIJ_AI_SESSION_DATA_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/zellij-ai-session}"
CONFIG_FILE="${ZELLIJ_AI_SESSION_CONFIG_FILE:-${ZELLIJ_CONFIG_FILE:-${XDG_CONFIG_HOME:-$HOME/.config}/zellij/config.kdl}}"
INDEXER_PATH="$BIN_DIR/zellij-ai-session-index"
PLUGIN_PATH="$DATA_DIR/zellij_ai_session_plugin.wasm"

command -v python3 >/dev/null || { echo "python3 is required to update Zellij config" >&2; exit 1; }

if [[ "$INSTALL_MODE" == auto ]]; then
    if [[ -f "$ROOT_DIR/Cargo.toml" && -d "$ROOT_DIR/crates" ]]; then
        INSTALL_MODE=source
    else
        INSTALL_MODE=download
    fi
fi

TEMP_DIR=""
cleanup() {
    if [[ -n "$TEMP_DIR" && -d "$TEMP_DIR" ]]; then
        rm -rf -- "$TEMP_DIR"
    fi
}
trap cleanup EXIT

if [[ "$INSTALL_MODE" == source ]]; then
    [[ -f "$ROOT_DIR/Cargo.toml" ]] || {
        echo "--from-source requires a source checkout" >&2
        exit 1
    }
    command -v cargo >/dev/null || { echo "cargo is required (install Rust via rustup.rs)" >&2; exit 1; }
    command -v rustup >/dev/null || { echo "rustup is required to install wasm32-wasip1" >&2; exit 1; }

    if ! rustup target list --installed | grep -qx wasm32-wasip1; then
        echo "Installing Rust target wasm32-wasip1..."
        rustup target add wasm32-wasip1
    fi

    echo "Building indexer from source..."
    cargo build --manifest-path "$ROOT_DIR/Cargo.toml" -p zellij-ai-session-index --release
    echo "Building Zellij plugin from source..."
    cargo build --manifest-path "$ROOT_DIR/Cargo.toml" \
        -p zellij-ai-session-plugin --target wasm32-wasip1 --features wasm --release

    SOURCE_INDEXER="$ROOT_DIR/target/release/zellij-ai-session-index"
    SOURCE_PLUGIN="$ROOT_DIR/target/wasm32-wasip1/release/zellij_ai_session_plugin.wasm"
else
    [[ "$REPOSITORY" == */* ]] || {
        echo "--repo must use OWNER/REPO format" >&2
        exit 2
    }
    case "$(uname -s):$(uname -m)" in
        Linux:x86_64|Linux:amd64) PLATFORM=linux-x86_64 ;;
        Linux:aarch64|Linux:arm64) PLATFORM=linux-aarch64 ;;
        Darwin:x86_64|Darwin:amd64) PLATFORM=macos-x86_64 ;;
        Darwin:arm64|Darwin:aarch64) PLATFORM=macos-aarch64 ;;
        *)
            echo "Unsupported platform: $(uname -s) $(uname -m)" >&2
            exit 1
            ;;
    esac

    if command -v curl >/dev/null 2>&1; then
        download_file() {
            curl -fL --retry 3 --connect-timeout 10 -o "$2" "$1"
        }
    elif command -v wget >/dev/null 2>&1; then
        download_file() {
            wget -O "$2" "$1"
        }
    else
        echo "curl or wget is required to download a GitHub Release" >&2
        exit 1
    fi

    if command -v sha256sum >/dev/null 2>&1; then
        file_sha256() { sha256sum "$1" | awk '{print $1}'; }
    elif command -v shasum >/dev/null 2>&1; then
        file_sha256() { shasum -a 256 "$1" | awk '{print $1}'; }
    else
        echo "sha256sum or shasum is required to verify downloads" >&2
        exit 1
    fi

    TEMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/zellij-ai-session.XXXXXX")
    INDEXER_ASSET="zellij-ai-session-index-$PLATFORM"
    PLUGIN_ASSET="zellij_ai_session_plugin.wasm"
    if [[ -n "$RELEASE_BASE_URL" ]]; then
        RELEASE_URL="${RELEASE_BASE_URL%/}"
    elif [[ "$RELEASE_VERSION" == latest ]]; then
        RELEASE_URL="https://github.com/$REPOSITORY/releases/latest/download"
    else
        RELEASE_URL="https://github.com/$REPOSITORY/releases/download/$RELEASE_VERSION"
    fi

    echo "Downloading zellij-ai-session $RELEASE_VERSION for $PLATFORM..."
    download_file "$RELEASE_URL/$INDEXER_ASSET" "$TEMP_DIR/$INDEXER_ASSET"
    download_file "$RELEASE_URL/$PLUGIN_ASSET" "$TEMP_DIR/$PLUGIN_ASSET"
    download_file "$RELEASE_URL/SHA256SUMS" "$TEMP_DIR/SHA256SUMS"

    verify_asset() {
        asset=$1
        expected=$(awk -v name="$asset" '$2 == name { print $1; exit }' "$TEMP_DIR/SHA256SUMS")
        [[ -n "$expected" ]] || { echo "No checksum found for $asset" >&2; exit 1; }
        actual=$(file_sha256 "$TEMP_DIR/$asset")
        [[ "$actual" == "$expected" ]] || {
            echo "Checksum verification failed for $asset" >&2
            exit 1
        }
    }
    verify_asset "$INDEXER_ASSET"
    verify_asset "$PLUGIN_ASSET"
    echo "Download checksums verified."
    SOURCE_INDEXER="$TEMP_DIR/$INDEXER_ASSET"
    SOURCE_PLUGIN="$TEMP_DIR/$PLUGIN_ASSET"
fi

mkdir -p "$BIN_DIR" "$DATA_DIR"
cp "$SOURCE_INDEXER" "$INDEXER_PATH"
cp "$SOURCE_PLUGIN" "$PLUGIN_PATH"
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
if [[ "$INSTALL_MODE" == source ]]; then
    echo "  source:  local checkout"
else
    echo "  release: $REPOSITORY/$RELEASE_VERSION"
fi
echo "  indexer: $INDEXER_PATH"
echo "  plugin:  $PLUGIN_PATH"
if ((INSTALL_KEYBIND)); then
    echo "  key:     $KEY"
    echo "  config:  $CONFIG_FILE"
    echo "Restart Zellij or reload its config, then press $KEY."
else
    echo "Keybind installation skipped (--no-keybind)."
fi
