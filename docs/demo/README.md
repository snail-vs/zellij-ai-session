# Demo recording

`docs/demo.gif` in the repository root is a recording of the real Zellij
plugin, not a hand-made mock-up. This directory holds the scripts that
produce it, so the demo can be regenerated whenever the interface changes.

## How it works

1. `build_data.py` writes a curated `snapshot.json` and `previews.json` with a
   fixed set of projects, agents and transcripts. The demo therefore does not
   change when the recording machine gains new sessions.
2. `indexer` is a fake indexer. The plugin asks it for a snapshot, for
   previews and for the command that resumes a session; this script answers
   all three from the curated data.
3. `record.py` starts Zellij inside tmux with an isolated configuration that
   loads the freshly built WASI plugin, sends the navigation keys and captures
   the terminal after every step.
4. `render.py` replays the captured frames, resolves the ANSI colors and draws
   them with Pillow. Only the plugin window is kept, the surrounding Zellij
   chrome is cropped away.
5. `assemble.py` turns the frames into `docs/demo.gif`.

## Requirements

- `zellij` on `PATH`
- `tmux`
- `ffmpeg`
- Python with `Pillow`:

```bash
python3 -m venv .venv
.venv/bin/pip install pillow
```

## Regenerating the demo

```bash
# 1. build the plugin that will be recorded
cargo build -p zellij-ai-session-plugin --target wasm32-wasip1 --features wasm --release

# 2. record, render and assemble
docs/demo/run.sh
```

`run.sh` writes the result to `docs/demo.gif`. The intermediate frames stay
in `docs/demo/work/` and can be deleted afterwards.
