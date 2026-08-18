"""Generate the Claude Code mascot pet ("Clawd") for claude-airou.

Source shape = the Claude Code welcome-screen glyphs:
     ▐▛███▜▌
    ▝▜█████▛▘
      ▘▘ ▝▝
Each glyph is a 2x2 quadrant block; a terminal cell is ~1:2, so one quadrant is
1 px wide x 2 px tall in square pixels. The missing quadrants in ▛ / ▜ on the top
row are the eyes.
"""
import json
import pathlib

W, H = 20, 16          # grid
BODY_TOP = 5           # grid row of the first body row
BODY_LEFT = 1          # grid col of mascot col 0

# 18-wide mascot rows (each quadrant row duplicated to 2 px tall), '#' body, 'E' eye hole
MASCOT = [
    "...############...",
    "...############...",
    "...##E######E##...",
    "...##E######E##...",
    ".################.",
    ".################.",
    "...############...",
    "...############...",   # <- shade row
    "....#.#....#.#....",   # feet
    "....#.#....#.#....",
]

PALETTE = {
    "o": "#D97757",  # Claude orange
    "s": "#BF6644",  # shade
    "e": "#1F1611",  # eyes
    "y": "#F5C518",  # sparkle
    "b": "#4C9AFF",  # thought dots / sweat
    "r": "#E5484D",  # alert
}

def blank():
    return [["."] * W for _ in range(H)]

def put(grid, row, col, ch):
    if 0 <= row < H and 0 <= col < W:
        grid[row][col] = ch

def base(eyes=True, body_dy=0):
    g = blank()
    for mr, line in enumerate(MASCOT):
        gr = BODY_TOP + mr + body_dy
        for mc, ch in enumerate(line):
            gc = BODY_LEFT + mc
            if ch == "#":
                shade = mr in (7, 8, 9)
                put(g, gr, gc, "s" if shade else "o")
            elif ch == "E":
                put(g, gr, gc, "e" if eyes else "o")
    return g

# eye centres (grid coords): left eye col = BODY_LEFT+5, right eye col = BODY_LEFT+12, rows BODY_TOP+2..3
LEFT_EYE_COL = BODY_LEFT + 5
RIGHT_EYE_COL = BODY_LEFT + 12
EYE_TOP = BODY_TOP + 2

def clear_eyes(g):
    for r in range(BODY_TOP, BODY_TOP + 4):
        for c in (LEFT_EYE_COL - 1, LEFT_EYE_COL, LEFT_EYE_COL + 1, RIGHT_EYE_COL - 1, RIGHT_EYE_COL, RIGHT_EYE_COL + 1):
            if g[r][c] == "e":
                g[r][c] = "o"

def eyes_at(g, rows, cols_left, cols_right, ch="e"):
    clear_eyes(g)
    for r in rows:
        for c in cols_left:
            put(g, r, c, ch)
        for c in cols_right:
            put(g, r, c, ch)

def rows_of(g):
    return ["".join(r) for r in g]

frames = {}

# idle: base, base, glance left, base, blink
idle_a = base()
glance = base(); eyes_at(glance, [EYE_TOP, EYE_TOP + 1], [LEFT_EYE_COL - 1], [RIGHT_EYE_COL - 1])
blink = base(eyes=False)
frames["idle"] = [rows_of(idle_a), rows_of(idle_a), rows_of(glance), rows_of(idle_a), rows_of(blink)]

# thinking: eyes look up-right (one row up, one col right) + blue dots up-right
th1 = base(); eyes_at(th1, [EYE_TOP - 1, EYE_TOP], [LEFT_EYE_COL + 1], [RIGHT_EYE_COL + 1])
put(th1, 2, 16, "b")
th2 = base(); eyes_at(th2, [EYE_TOP - 1, EYE_TOP], [LEFT_EYE_COL + 1], [RIGHT_EYE_COL + 1])
put(th2, 2, 15, "b"); put(th2, 1, 17, "b"); put(th2, 0, 19, "b")
frames["thinking"] = [rows_of(th1), rows_of(th2)]

# working: squint (1px tall eyes on lower row), feet shuffle
wk1 = base(); eyes_at(wk1, [EYE_TOP + 1], [LEFT_EYE_COL - 1, LEFT_EYE_COL, LEFT_EYE_COL + 1], [RIGHT_EYE_COL - 1, RIGHT_EYE_COL, RIGHT_EYE_COL + 1])
wk2 = base(); eyes_at(wk2, [EYE_TOP + 1], [LEFT_EYE_COL - 1, LEFT_EYE_COL, LEFT_EYE_COL + 1], [RIGHT_EYE_COL - 1, RIGHT_EYE_COL, RIGHT_EYE_COL + 1])
# frame 2: left feet lift (remove bottom foot row on the left pair), right feet stay
foot_row = BODY_TOP + 9
for c in (BODY_LEFT + 4, BODY_LEFT + 6):
    put(wk2, foot_row, c, ".")
    put(wk2, foot_row - 2, c, "s")  # foot tucks up against the body row (visible shuffle)
frames["working"] = [rows_of(wk1), rows_of(wk2)]

# waiting_approval: wide 2x3 eyes + red "!" floating up-right (3-row bar, gap, dot)
def wide_eyes(g):
    eyes_at(g, [EYE_TOP - 1, EYE_TOP, EYE_TOP + 1], [LEFT_EYE_COL - 1, LEFT_EYE_COL], [RIGHT_EYE_COL, RIGHT_EYE_COL + 1])
wa1 = base(); wide_eyes(wa1)
for r in (0, 1, 2): put(wa1, r, 18, "r"); put(wa1, r, 19, "r")
put(wa1, 4, 18, "r"); put(wa1, 4, 19, "r")
wa2 = base(); wide_eyes(wa2)
for r in (1, 2, 3): put(wa2, r, 18, "r"); put(wa2, r, 19, "r")
put(wa2, 5, 18, "r"); put(wa2, 5, 19, "r")
frames["waiting_approval"] = [rows_of(wa1), rows_of(wa2)]

# needs_input: half-lidded (1px, lower row) + yellow "?" up-right
def question(g, dy=0):
    q = ["yyy", "..y", ".yy", "...", ".y."]
    for r, line in enumerate(q):
        for c, ch in enumerate(line):
            if ch == "y":
                put(g, r + dy, 17 + c, "y")
ni1 = base(); eyes_at(ni1, [EYE_TOP + 1], [LEFT_EYE_COL], [RIGHT_EYE_COL]); question(ni1, 0)
ni2 = base(); eyes_at(ni2, [EYE_TOP + 1], [LEFT_EYE_COL], [RIGHT_EYE_COL]); question(ni2, 1)
frames["needs_input"] = [rows_of(ni1), rows_of(ni2)]

# done: ^^ eyes + sparkles
def happy_eyes(g):
    clear_eyes(g)
    for col in (LEFT_EYE_COL, RIGHT_EYE_COL):
        put(g, EYE_TOP, col, "e")
        put(g, EYE_TOP + 1, col - 1, "e")
        put(g, EYE_TOP + 1, col + 1, "e")
d1 = base(); happy_eyes(d1)
put(d1, 0, 1, "y"); put(d1, 1, 18, "y"); put(d1, 3, 0, "y")
d2 = base(); happy_eyes(d2)
put(d2, 1, 2, "y"); put(d2, 0, 17, "y"); put(d2, 3, 19, "y")
frames["done"] = [rows_of(d1), rows_of(d2)]

# error: x eyes + sweat drop
def x_eyes(g):
    clear_eyes(g)
    for col in (LEFT_EYE_COL, RIGHT_EYE_COL):
        put(g, EYE_TOP - 1, col - 1, "e"); put(g, EYE_TOP - 1, col + 1, "e")
        put(g, EYE_TOP, col, "e")
        put(g, EYE_TOP + 1, col - 1, "e"); put(g, EYE_TOP + 1, col + 1, "e")
e1 = base(); x_eyes(e1); put(e1, 3, 18, "b"); put(e1, 4, 18, "b")
e2 = base(); x_eyes(e2); put(e2, 4, 18, "b"); put(e2, 5, 18, "b"); put(e2, 5, 19, "b")
frames["error"] = [rows_of(e1), rows_of(e2)]

# hello: hop
h1 = base()
h2 = base(body_dy=-1)
frames["hello"] = [rows_of(h1), rows_of(h2), rows_of(h1)]

pet = {
    "id": "clawd-claude",
    "name": "Clawd",
    "species": "claude",
    "description": "The little orange creature from the Claude Code welcome screen, now supervising from your desktop.",
    "author": "claude-airou",
    "fps": 3,
    "palette": PALETTE,
    "phrases": {
        "pet": [
            "I read the whole file. Promise.",
            "Have you tried /clear?",
            "Context: fine. Mood: fine.",
            "Ship it?",
            "…thinking harder.",
            "It compiled on my machine.",
        ]
    },
    "frames": frames,
}

def dump(obj, indent=0):
    pad = "  " * indent
    if isinstance(obj, dict):
        items = [f'{pad}  {json.dumps(k)}: {dump(v, indent + 1)}' for k, v in obj.items()]
        return "{\n" + ",\n".join(items) + f"\n{pad}}}"
    if isinstance(obj, list):
        if obj and all(isinstance(x, str) for x in obj):
            if all(len(x) == W for x in obj):
                return "[\n" + ",\n".join(f'{pad}  {json.dumps(x)}' for x in obj) + f"\n{pad}]"
            return json.dumps(obj, ensure_ascii=False)
        return "[\n" + ",\n".join(f'{pad}  {dump(x, indent + 1)}' for x in obj) + f"\n{pad}]"
    return json.dumps(obj, ensure_ascii=False)

out = pathlib.Path(__file__).resolve().parent.parent / "Sources" / "ClaudeAirou" / "Resources" / "pets" / "clawd-claude.json"
out.write_text(dump(pet) + "\n")
print("wrote", out)
