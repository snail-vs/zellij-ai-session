#!/usr/bin/env python3
"""Assemble the captured frames into the demo GIF."""
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", ".."))
PNG = os.path.join(HERE, "work", "png")
OUT = os.path.join(REPO, "docs", "demo.gif")

# dwell time per frame in seconds
DWELL = [
    1.6,   # overview
    1.4,   # OpenCode 2.0 session
    1.4,   # OMP support session
    1.4,   # omp session
    1.4,   # typing project session
    1.4,   # omp session in typing
    1.6,   # opencode session in gitops
    1.0,   # search view
    1.8,   # search results
    1.2,   # back to the tree
    1.0,   # tree
]


def main():
    names = sorted(n for n in os.listdir(PNG) if n.endswith(".png"))
    if len(names) != len(DWELL):
        raise SystemExit(f"expected {len(DWELL)} frames, found {len(names)}")
    listing = os.path.join(HERE, "work", "concat.txt")
    with open(listing, "w", encoding="utf-8") as handle:
        for name, dwell in zip(names, DWELL):
            handle.write(f"file '{os.path.join(PNG, name)}'\n")
            handle.write(f"duration {dwell}\n")
        # The concat demuxer needs the last file repeated.
        handle.write(f"file '{os.path.join(PNG, names[-1])}'\n")

    command = [
        "ffmpeg", "-y", "-hide_banner", "-loglevel", "error",
        "-f", "concat", "-safe", "0", "-i", listing,
        "-vf", "fps=12,scale=iw:ih:flags=lanczos,split[a][b];[a]palettegen=max_colors=192[p];[b][p]paletteuse=dither=bayer:bayer_scale=3",
        "-loop", "0", "-final_delay", "0",
        OUT,
    ]
    subprocess.run(command, check=True)
    size = os.path.getsize(OUT) / 1024 / 1024
    print(f"{OUT}  {size:.2f} MB")
    return 0


if __name__ == "__main__":
    sys.exit(main())
