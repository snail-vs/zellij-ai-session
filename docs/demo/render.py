#!/usr/bin/env python3
"""Turn the captured terminal frames into PNG images.

`tmux capture-pane -e` writes one line per terminal row containing only SGR
escape sequences, so the frames are parsed directly instead of being replayed
through a terminal emulator. Only the plugin window is kept, the surrounding
Zellij chrome is cropped away.
"""
import os
import re
import sys

from PIL import Image, ImageDraw, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
WORK = os.path.join(HERE, "work")
FRAMES = os.path.join(WORK, "frames")
OUT = os.path.join(WORK, "png")

COLS, ROWS = 240, 46
CELL_W, CELL_H = 8, 17
PAD = 14

LATIN_FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"
CJK_FONT = "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc"

DEFAULT_FG = (222, 226, 235)
DEFAULT_BG = (24, 26, 32)

BASIC = {
    30: (40, 42, 54), 31: (255, 85, 85), 32: (80, 250, 123), 33: (241, 250, 140),
    34: (98, 114, 164), 35: (255, 121, 198), 36: (139, 233, 253), 37: (248, 248, 242),
    90: (98, 106, 130), 91: (255, 110, 110), 92: (105, 255, 140), 93: (255, 250, 160),
    94: (120, 140, 200), 95: (255, 145, 215), 96: (160, 245, 255), 97: (255, 255, 255),
}

SGR = re.compile(r"\x1b\[([0-9;]*)m")


def xterm256(index):
    if index < 16:
        base = [
            (40, 42, 54), (255, 85, 85), (80, 250, 123), (241, 250, 140),
            (98, 114, 164), (255, 121, 198), (139, 233, 253), (248, 248, 242),
            (98, 106, 130), (255, 110, 110), (105, 255, 140), (255, 250, 160),
            (120, 140, 200), (255, 145, 215), (160, 245, 255), (255, 255, 255),
        ]
        return base[index]
    if index < 232:
        index -= 16
        levels = [0, 95, 135, 175, 215, 255]
        return (levels[index // 36], levels[(index // 6) % 6], levels[index % 6])
    grey = 8 + (index - 232) * 10
    return (grey, grey, grey)


def parse_line(line):
    """Split a captured line into cells of (char, fg, bg, bold).

    A double width character covers two terminal columns, so it is followed by
    an empty continuation cell.
    """
    cells = []
    fg, bg, bold = DEFAULT_FG, DEFAULT_BG, False
    position = 0
    for match in SGR.finditer(line):
        for char in line[position:match.start()]:
            cells.append((char, fg, bg, bold))
            if is_wide(char):
                cells.append(("", fg, bg, bold))
        position = match.end()
        codes = [int(code) for code in match.group(1).split(";") if code != ""] or [0]
        index = 0
        while index < len(codes):
            code = codes[index]
            if code == 0:
                fg, bg, bold = DEFAULT_FG, DEFAULT_BG, False
            elif code == 1:
                bold = True
            elif code == 22:
                bold = False
            elif code == 39:
                fg = DEFAULT_FG
            elif code == 49:
                bg = DEFAULT_BG
            elif code in BASIC:
                fg = BASIC[code]
            elif 40 <= code <= 47:
                bg = BASIC[code - 10]
            elif code in (38, 48):
                if index + 2 < len(codes) and codes[index + 1] == 5:
                    color = xterm256(codes[index + 2])
                    index += 2
                elif index + 2 < len(codes) and codes[index + 1] == 2:
                    color = tuple(codes[index + 2:index + 5])
                    index += 4
                else:
                    color = DEFAULT_FG
                if code == 38:
                    fg = color
                else:
                    bg = color
            index += 1
    for char in line[position:]:
        cells.append((char, fg, bg, bold))
        if is_wide(char):
            cells.append(("", fg, bg, bold))
    return cells


def parse(path):
    with open(path, encoding="utf-8") as handle:
        raw = handle.read()
    lines = raw.split("\n")
    grid = []
    for line in lines[:ROWS]:
        cells = parse_line(line)
        cells += [(" ", DEFAULT_FG, DEFAULT_BG, False)] * (COLS - len(cells))
        grid.append(cells[:COLS])
    while len(grid) < ROWS:
        grid.append([(" ", DEFAULT_FG, DEFAULT_BG, False)] * COLS)
    return grid


def is_wide(char):
    code = ord(char)
    return (
        0x1100 <= code <= 0x115F
        or 0x2E80 <= code <= 0xA4CF
        or 0xAC00 <= code <= 0xD7A3
        or 0xF900 <= code <= 0xFAFF
        or 0xFE30 <= code <= 0xFE4F
        or 0xFF00 <= code <= 0xFF60
        or 0xFFE0 <= code <= 0xFFE6
        or 0x1F300 <= code <= 0x1FAFF
        or 0x20000 <= code <= 0x3FFFD
    )


def load_fonts():
    latin = ImageFont.truetype(LATIN_FONT, CELL_H - 4)
    cjk = ImageFont.truetype(CJK_FONT, CELL_H - 2)
    advance = cjk.getlength("中")
    if advance > 0:
        size = max(6, int((CELL_H - 2) * (2 * CELL_W) / advance))
        cjk = ImageFont.truetype(CJK_FONT, size)
    return latin, cjk


def pane_box(grid):
    """Bounding box of the plugin window inside the terminal."""
    top = left = right = bottom = None
    for y, row in enumerate(grid):
        text = "".join(cell[0] for cell in row)
        if "┌" in text and "┐" in text:
            top = y
            left = text.index("┌")
            right = text.rindex("┐") + 1
            break
    if top is None:
        return 0, 0, COLS, ROWS
    for y in range(top, len(grid)):
        text = "".join(cell[0] for cell in grid[y])
        if "└" in text and "┘" in text:
            bottom = y + 1
            right = max(right, text.rindex("┘") + 1)
            break
    if bottom is None:
        bottom = min(ROWS, top + 30)
    return top, left, right, bottom


def render(grid, box, fonts):
    top, left, right, bottom = box
    width = (right - left) * CELL_W + PAD * 2
    height = (bottom - top) * CELL_H + PAD * 2
    image = Image.new("RGB", (width, height), DEFAULT_BG)
    draw = ImageDraw.Draw(image)
    latin, cjk = fonts

    for y in range(top, bottom):
        for x in range(left, right):
            char, fg, bg, bold = grid[y][x]
            px = PAD + (x - left) * CELL_W
            py = PAD + (y - top) * CELL_H
            if bg != DEFAULT_BG:
                draw.rectangle([px, py, px + CELL_W, py + CELL_H], fill=bg)
            if not char.strip():
                continue
            if is_wide(char):
                font = cjk
                color = fg
            else:
                font = latin
                color = fg
            if bold and color == DEFAULT_FG:
                color = (255, 255, 255)
            draw.text((px, py), char, font=font, fill=color)
    return image


def main():
    os.makedirs(OUT, exist_ok=True)
    for name in os.listdir(OUT):
        os.remove(os.path.join(OUT, name))
    fonts = load_fonts()
    for name in sorted(n for n in os.listdir(FRAMES) if n.endswith(".txt")):
        grid = parse(os.path.join(FRAMES, name))
        box = pane_box(grid)
        image = render(grid, box, fonts)
        path = os.path.join(OUT, name.replace(".txt", ".png"))
        image.save(path)
        print(f"{name} -> {os.path.basename(path)} {image.size} box={box}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
