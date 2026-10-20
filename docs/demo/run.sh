#!/usr/bin/env bash
# Record the demo and write it to docs/demo.gif.
#
# Requirements: zellij, tmux, ffmpeg and Python with Pillow.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
PYTHON="${PYTHON:-python3}"

# Pillow is the only extra dependency; prefer a local virtual environment.
if ! "$PYTHON" -c "import PIL" >/dev/null 2>&1; then
    if [ -x "$REPO/.venv/bin/python" ]; then
        PYTHON="$REPO/.venv/bin/python"
    else
        echo "Pillow is missing. Install it with:" >&2
        echo "  python3 -m venv $REPO/.venv && $REPO/.venv/bin/pip install pillow" >&2
        echo "then re-run with PYTHON=$REPO/.venv/bin/python $0" >&2
        exit 1
    fi
fi

PLUGIN="$REPO/target/wasm32-wasip1/release/zellij_ai_session_plugin.wasm"
if [ ! -f "$PLUGIN" ]; then
    echo "build the plugin first:" >&2
    echo "  cargo build -p zellij-ai-session-plugin --target wasm32-wasip1 --features wasm --release" >&2
    exit 1
fi

echo "Building the demo data..."
"$PYTHON" "$HERE/build_data.py"

echo "Recording the plugin inside tmux..."
"$PYTHON" "$HERE/record.py"

echo "Drawing the frames..."
"$PYTHON" "$HERE/render.py"

echo "Assembling the GIF..."
"$PYTHON" "$HERE/assemble.py"

echo "Done: $REPO/docs/demo.gif"
