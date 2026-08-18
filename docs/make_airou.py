"""Generate the Airou pet — a Monster-Hunter-Felyne-inspired hunting companion — for claude-airou.

Checked against reference images (Capcom's "Airou from the Monster Hunter" art, the Fuwatama /
deformed plushes, the MegaHouse figure, the wiki render): cream fur with brown "points" — an
triangle mask widest around the muzzle and narrowing up between the eyes, brown ear backs with pink insides, brown
paw / feet tips and tail tuft — big round blue eyes sitting on the mask edges, a small nose over
an "ω" mouth, whiskers, a chubby body. Palico gear: a leather vest with belt and pouch, and the
signature paw hammer planted on the ground beside it.

Silhouette pieces are painted into a mask, the outline is derived automatically (mask pixels
touching transparency), then features and gear are overlaid. States only change eyes / mouth /
props / one paw, so the body stays put.
Run:  python3 docs/make_airou.py && claude-airou validate Sources/ClaudeAirou/Resources/pets/airou-felyne.json
"""
import json
import pathlib

W = H = 28

PALETTE = {
    "k": "#4A3020",  # outline (dark brown)
    "c": "#F5ECD8",  # cream fur
    "t": "#A6754A",  # brown points: face mask, ear backs, paw/feet tips, tail tuft
    "p": "#EFA6BC",  # pink ear insides / nose / hammer pads
    "e": "#1F1611",  # eye rims and pupils
    "i": "#3FA7D6",  # blue iris
    "w": "#FFFFFF",  # eye highlight
    "v": "#C4854A",  # leather vest
    "l": "#E9C58C",  # belt / pouch / vest trim
    "o": "#8B5E34",  # hammer handle
    "y": "#F5C518",  # sparkle / question mark
    "b": "#4C9AFF",  # thought dots / sweat
    "r": "#E5484D",  # alert "!"
}

# ---------------------------------------------------------------- helpers

def blank():
    return [[False] * W for _ in range(H)]

def put(grid, x, y, ch):
    if 0 <= x < W and 0 <= y < H:
        grid[y][x] = ch

def ellipse(mask, cx, cy, rx, ry):
    for y in range(H):
        for x in range(W):
            if ((x - cx) / rx) ** 2 + ((y - cy) / ry) ** 2 <= 1.0:
                mask[y][x] = True

def rect(mask, x0, y0, x1, y1):
    for y in range(y0, y1 + 1):
        for x in range(x0, x1 + 1):
            if 0 <= x < W and 0 <= y < H:
                mask[y][x] = True

def triangle(mask, a, b, c):
    def sign(p1, p2, p3):
        return (p1[0] - p3[0]) * (p2[1] - p3[1]) - (p2[0] - p3[0]) * (p1[1] - p3[1])
    for y in range(H):
        for x in range(W):
            p = (x + 0.5, y + 0.5)
            d1, d2, d3 = sign(p, a, b), sign(p, b, c), sign(p, c, a)
            neg = d1 < 0 or d2 < 0 or d3 < 0
            pos = d1 > 0 or d2 > 0 or d3 > 0
            if not (neg and pos):
                mask[y][x] = True

def render(mask, fill="c"):
    """Mask -> character grid with an automatic 4-neighbour outline."""
    grid = [["."] * W for _ in range(H)]
    for y in range(H):
        for x in range(W):
            if not mask[y][x]:
                continue
            edge = False
            for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                nx, ny = x + dx, y + dy
                if nx < 0 or ny < 0 or nx >= W or ny >= H or not mask[ny][nx]:
                    edge = True
                    break
            grid[y][x] = "k" if edge else fill
    return grid

def paint_inside(g, mask, ch, only=("c",)):
    for y in range(H):
        for x in range(W):
            if mask[y][x] and g[y][x] in only:
                g[y][x] = ch

def line(g, x0, y0, x1, y1, ch):
    """Bresenham line."""
    dx, dy = abs(x1 - x0), -abs(y1 - y0)
    sx, sy = (1 if x0 < x1 else -1), (1 if y0 < y1 else -1)
    err = dx + dy
    while True:
        put(g, x0, y0, ch)
        if x0 == x1 and y0 == y1:
            break
        e2 = 2 * err
        if e2 >= dy: err += dy; x0 += sx
        if e2 <= dx: err += dx; y0 += sy

# ---------------------------------------------------------------- body geometry (28x28)

HEAD = (13.5, 11.2, 10.4, 8.0)                     # cx, cy, rx, ry  -> x 3..24, y 3..19
LEFT_EAR = ((5.0, 0.0), (0.8, 8.4), (11.4, 5.4))
RIGHT_EAR = ((22.0, 0.0), (26.2, 8.4), (15.6, 5.4))
LEFT_CHEEK = (5.4, 14.6, 2.9, 2.6)
RIGHT_CHEEK = (21.6, 14.6, 2.9, 2.6)
TORSO = (8, 19, 19, 24)
LEFT_PAW = (6, 20, 7, 23)
RIGHT_PAW = (20, 20, 21, 23)
LEFT_FOOT = (8, 25, 11, 27)
RIGHT_FOOT = (16, 25, 19, 27)
EYE_LEFT_X, EYE_RIGHT_X, EYE_Y = 6, 16, 9          # 6x6 eyes
MASK_TRIANGLE = ((13.5, 7.6), (6.0, 18.4), (21.0, 18.4))   # apex between the eyes (not up to the forehead), base around the muzzle

def body_mask():
    m = blank()
    ellipse(m, *HEAD)
    ellipse(m, *LEFT_CHEEK)
    ellipse(m, *RIGHT_CHEEK)
    triangle(m, *LEFT_EAR)
    triangle(m, *RIGHT_EAR)
    rect(m, *TORSO)
    rect(m, *LEFT_PAW)
    rect(m, *RIGHT_PAW)
    rect(m, *LEFT_FOOT)
    rect(m, *RIGHT_FOOT)
    return m

def eye(g, x0, y0, look_dx=0, look_dy=0):
    """6x6 round eye: dark rim, blue iris, 2x2 pupil, highlights. Pupil offset by (look_dx, look_dy)."""
    for x in range(x0, x0 + 6):
        for y in range(y0, y0 + 6):
            put(g, x, y, "i")
    for (x, y) in ((x0, y0), (x0 + 5, y0), (x0, y0 + 5), (x0 + 5, y0 + 5)):
        put(g, x, y, "c" if g[y][x] not in ("t",) else "t")   # rounded corners keep the fur behind
    for x in range(x0 + 1, x0 + 5):
        put(g, x, y0, "e"); put(g, x, y0 + 5, "e")
    for y in range(y0 + 1, y0 + 5):
        put(g, x0, y, "e"); put(g, x0 + 5, y, "e")
    px, py = x0 + 2 + look_dx, y0 + 2 + look_dy
    for x in (px, px + 1):
        for y in (py, py + 1):
            put(g, x, y, "e")
    put(g, x0 + 1, y0 + 1, "w"); put(g, x0 + 2, y0 + 1, "w"); put(g, x0 + 4, y0 + 4, "w")

def base(tail=0, raised_paw=None, hammer=True):
    g = render(body_mask())

    # ears: brown backs, pink insides
    ears = blank(); triangle(ears, *LEFT_EAR); triangle(ears, *RIGHT_EAR)
    paint_inside(g, ears, "t")
    inner = blank()
    triangle(inner, (5.2, 2.6), (3.0, 7.0), (8.6, 5.6))
    triangle(inner, (21.8, 2.6), (24.0, 7.0), (18.4, 5.6))
    paint_inside(g, inner, "p", only=("t",))

    # brown face mask: a triangle that is widest around the muzzle and narrows up to the forehead
    mask = blank(); triangle(mask, *MASK_TRIANGLE)
    paint_inside(g, mask, "t")

    # eyes on the mask edges
    eye(g, EYE_LEFT_X, EYE_Y); eye(g, EYE_RIGHT_X, EYE_Y)

    # nose (pink) and "ω" mouth inside the brown muzzle
    put(g, 13, 14, "p"); put(g, 14, 14, "p"); put(g, 13, 15, "p"); put(g, 14, 15, "p")
    for (x, y) in ((11, 16), (13, 16), (15, 16), (12, 17), (14, 17)):
        put(g, x, y, "k")

    # whiskers, two per side, from the cheeks
    for (x, y) in ((1, 12), (2, 12), (1, 14), (2, 14), (25, 12), (26, 12), (25, 14), (26, 14)):
        put(g, x, y, "k")
    for (x, y) in ((4, 16), (5, 16), (22, 16), (23, 16)):
        put(g, x, y, "k")

    # leather vest with belt and a hip pouch (Palico gear)
    for x in range(9, 19):
        for y in range(19, 24):
            if g[y][x] == "c":
                put(g, x, y, "v")
    for x in (12, 13, 14, 15):
        put(g, x, 19, "c")                    # neck opening
    put(g, 12, 20, "c"); put(g, 15, 20, "c")
    for x in range(9, 19):
        put(g, x, 23, "l")                    # belt
    put(g, 9, 21, "l"); put(g, 10, 21, "l"); put(g, 9, 22, "l"); put(g, 10, 22, "l")   # pouch
    put(g, 9, 21, "k")

    # brown paw / feet tips
    for (x, y) in ((6, 23), (7, 23), (20, 23), (21, 23)):
        put(g, x, y, "t")
    for x in (8, 9, 10, 11, 16, 17, 18, 19):
        put(g, x, 26, "t")

    # long thin tail curling up on the right, fluffy brown tuft; two poses
    if tail == 0:
        for (x, y) in ((20, 25), (21, 26), (22, 25), (23, 24), (24, 23), (24, 22), (24, 21)):
            put(g, x, y, "k")
        for dx in range(3):
            for dy in range(3):
                put(g, 23 + dx, 18 + dy, "t")
        for (x, y) in ((23, 18), (25, 18), (23, 20), (25, 20)):
            put(g, x, y, "k")
    else:
        for (x, y) in ((20, 25), (21, 26), (22, 25), (23, 24), (24, 23), (25, 22), (25, 21)):
            put(g, x, y, "k")
        for dx in range(3):
            for dy in range(3):
                put(g, 24 + dx, 18 + dy, "t")
        for (x, y) in ((24, 18), (26, 18), (24, 20), (26, 20)):
            put(g, x, y, "k")

    # signature paw hammer planted on the ground beside the left foot, handle angled up into the paw
    if hammer:
        # head: 6x6 paw (pad + three toes) at the bottom-left, brown with pink pads, outlined
        for x in range(0, 6):
            for y in range(23, 28):
                put(g, x, y, "t")
        for (x, y) in ((0, 22), (1, 22), (2, 22), (3, 22), (4, 22), (5, 22)):
            put(g, x, y, "t")
        for (x, y) in ((0, 21), (2, 21), (4, 21)):
            put(g, x, y, "t")                       # toe tips
        for (x, y) in ((1, 24), (2, 24), (3, 24), (4, 24), (1, 25), (2, 25), (3, 25), (4, 25), (2, 26), (3, 26)):
            put(g, x, y, "p")                       # big pad
        for (x, y) in ((0, 23), (2, 23), (4, 23)):
            put(g, x, y, "p")                       # toe pads
        for (x, y) in ((0, 20), (2, 20), (4, 20), (1, 21), (3, 21), (5, 21), (6, 22), (6, 23), (6, 24), (6, 25), (6, 26), (6, 27), (0, 27), (5, 27)):
            if g[y][x] == ".":
                put(g, x, y, "k")                   # outline where free
        # handle from the head up into the left paw
        put(g, 3, 20, "o"); put(g, 4, 20, "o"); put(g, 5, 19, "o"); put(g, 5, 20, "o")

    # raised right paw (hello): 5x5 fist beside the head with an arm down to the shoulder
    if raised_paw is not None:
        px, py = raised_paw
        for dx in range(5):
            for dy in range(5):
                put(g, px + dx, py + dy, "k")
        for dx in (1, 2, 3):
            for dy in (1, 2, 3):
                put(g, px + dx, py + dy, "c")
        for dx in (1, 2, 3):
            put(g, px + dx, py + 3, "t")
        for y in range(py + 5, 20):
            put(g, px, y, "k"); put(g, px + 1, y, "c"); put(g, px + 2, y, "c"); put(g, px + 3, y, "k")
        for (x, y) in ((20, 20), (21, 20), (20, 21), (21, 21), (20, 22), (21, 22), (20, 23), (21, 23)):
            put(g, x, y, "c")
        put(g, 20, 24, "k"); put(g, 21, 24, "k")
    return g

def clear_eyes(g):
    for x0 in (EYE_LEFT_X, EYE_RIGHT_X):
        for x in range(x0, x0 + 6):
            for y in range(EYE_Y - 1, EYE_Y + 7):
                if g[y][x] in ("e", "i", "w"):
                    # restore fur/mask behind: mask triangle decides
                    g[y][x] = "t" if in_mask(x, y) else "c"

_MASK = blank(); triangle(_MASK, *MASK_TRIANGLE)
def in_mask(x, y):
    return _MASK[y][x]

def clear_mouth(g):
    for (x, y) in ((11, 16), (13, 16), (15, 16), (12, 17), (14, 17)):
        put(g, x, y, "t" if in_mask(x, y) else "c")

def rows(g):
    return ["".join(r) for r in g]

frames = {}

# idle: tail sways, one blink (eyes closed = curved dark lines)
i0 = base(tail=0); i1 = base(tail=1); i2 = base(tail=0)
blink = base(tail=0); clear_eyes(blink)
for x0 in (EYE_LEFT_X, EYE_RIGHT_X):
    put(blink, x0, EYE_Y + 2, "e"); put(blink, x0 + 5, EYE_Y + 2, "e")
    for x in range(x0 + 1, x0 + 5):
        put(blink, x, EYE_Y + 3, "e")
frames["idle"] = [rows(i0), rows(i1), rows(i2), rows(blink)]

# thinking: pupils drift up-right + blue dots
def thinking(dots):
    g = base(tail=0); clear_eyes(g)
    eye(g, EYE_LEFT_X, EYE_Y, look_dx=1, look_dy=-1); eye(g, EYE_RIGHT_X, EYE_Y, look_dx=1, look_dy=-1)
    for (x, y) in dots:
        put(g, x, y, "b")
    return g
frames["thinking"] = [rows(thinking([(27, 5)])), rows(thinking([(26, 6), (27, 4), (27, 2)]))]

# working: focused — upper lids drawn down over the eyes + slanted brows, left paw nudges up
def working(paw_up):
    g = base(tail=0)
    for x0 in (EYE_LEFT_X, EYE_RIGHT_X):
        for x in range(x0, x0 + 6):
            for y in (EYE_Y, EYE_Y + 1):
                put(g, x, y, "t" if in_mask(x, y) else "c")
        for x in range(x0 + 1, x0 + 5):
            put(g, x, EYE_Y + 2, "e")   # lid line
        put(g, x0, EYE_Y + 3, "e"); put(g, x0 + 5, EYE_Y + 3, "e")
    put(g, 6, 7, "k"); put(g, 7, 8, "k"); put(g, 21, 7, "k"); put(g, 20, 8, "k")
    if paw_up:
        for y in (20, 21, 22, 23):
            put(g, 20, y, "c"); put(g, 21, y, "c")
        put(g, 20, 19, "k"); put(g, 21, 19, "k"); put(g, 20, 20, "k"); put(g, 21, 20, "c"); put(g, 20, 21, "t"); put(g, 21, 21, "t"); put(g, 20, 22, "k"); put(g, 21, 22, "k")
    return g
frames["working"] = [rows(working(False)), rows(working(True))]

# waiting_approval: big pupils, small "o" mouth, red "!" floating clear of the right ear
def waiting(shift):
    g = base(tail=0); clear_eyes(g); clear_mouth(g)
    for x0 in (EYE_LEFT_X, EYE_RIGHT_X):
        eye(g, x0, EYE_Y)
        for x in range(x0 + 2, x0 + 5):
            for y in range(EYE_Y + 2, EYE_Y + 5):
                put(g, x, y, "e")           # dilated pupil
        put(g, x0 + 1, EYE_Y + 1, "w"); put(g, x0 + 2, EYE_Y + 1, "w")
    put(g, 13, 16, "e"); put(g, 14, 16, "e"); put(g, 13, 17, "e"); put(g, 14, 17, "e")
    for y in range(0 + shift, 4 + shift):
        put(g, 26, y, "r"); put(g, 27, y, "r")
    put(g, 26, 5 + shift, "r"); put(g, 27, 5 + shift, "r")
    return g
frames["waiting_approval"] = [rows(waiting(0)), rows(waiting(1))]

# needs_input: half-lidded + yellow "?"
def needs_input(dy):
    g = base(tail=0)
    for x0 in (EYE_LEFT_X, EYE_RIGHT_X):
        for x in range(x0, x0 + 6):
            for y in (EYE_Y, EYE_Y + 1):
                put(g, x, y, "t" if in_mask(x, y) else "c")
        for x in range(x0 + 1, x0 + 5):
            put(g, x, EYE_Y + 2, "e")
    q = ["yyyy", "...y", "..yy", "....", "..y."]
    for r, ln in enumerate(q):
        for c, ch in enumerate(ln):
            if ch == "y":
                put(g, 24 + c, r + dy, "y")
    return g
frames["needs_input"] = [rows(needs_input(0)), rows(needs_input(1))]

# done: ^^ eyes + wide open smile + sparkles
def done(alt):
    g = base(tail=1 if alt else 0); clear_eyes(g); clear_mouth(g)
    for x0 in (EYE_LEFT_X, EYE_RIGHT_X):
        put(g, x0 + 2, EYE_Y + 2, "e"); put(g, x0 + 3, EYE_Y + 2, "e")
        put(g, x0 + 1, EYE_Y + 3, "e"); put(g, x0 + 4, EYE_Y + 3, "e")
        put(g, x0, EYE_Y + 4, "e"); put(g, x0 + 5, EYE_Y + 4, "e")
    put(g, 11, 16, "k"); put(g, 16, 16, "k")
    for x in range(12, 16):
        put(g, x, 17, "k")
    for (x, y) in ([(1, 10), (26, 9), (2, 18)] if not alt else [(0, 13), (27, 7), (1, 16)]):
        put(g, x, y, "y")
    return g
frames["done"] = [rows(done(False)), rows(done(True))]

# error: x x eyes + frown + sweat drop
def error(drop_dy):
    g = base(tail=0); clear_eyes(g); clear_mouth(g)
    for x0 in (EYE_LEFT_X, EYE_RIGHT_X):
        for d in range(6):
            put(g, x0 + d, EYE_Y + d, "e"); put(g, x0 + 5 - d, EYE_Y + d, "e")
    put(g, 11, 17, "k"); put(g, 16, 17, "k")
    for x in range(12, 16):
        put(g, x, 16, "k")
    put(g, 26, 10 + drop_dy, "b"); put(g, 26, 11 + drop_dy, "b")
    return g
frames["error"] = [rows(error(0)), rows(error(2))]

# hello: right paw raised and waving (the hammer stays on the left shoulder)
frames["hello"] = [rows(base(tail=0, raised_paw=(23, 11))), rows(base(tail=1, raised_paw=(23, 9)))]

pet = {
    "id": "airou-felyne",
    "name": "Airou",
    "species": "felyne",
    "description": "A Felyne hunting companion: cream fur with brown points and blue eyes, leather vest, paw hammer on the shoulder. Carves your builds for materials, nya.",
    "author": "claude-airou",
    "fps": 3,
    "palette": PALETTE,
    "phrases": {
        "pet": [
            "Nya! Quest accepted, meowster.",
            "Carving the logs for materials…",
            "That build had a weak spot, nya.",
            "Meowster, the compiler flinched!",
            "Palico Rally: +10 to morale.",
            "Time to sharpen the linter.",
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

out = pathlib.Path(__file__).resolve().parent.parent / "Sources" / "ClaudeAirou" / "Resources" / "pets" / "airou-felyne.json"
out.write_text(dump(pet) + "\n")
print("wrote", out)
