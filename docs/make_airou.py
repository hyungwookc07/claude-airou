"""Generate the Airou pet (Monster-Hunter-Felyne-inspired upright cat) for claude-airou.

Silhouette pieces are painted into a mask, the outline is derived automatically (mask pixels
touching transparency), then features (ears, eyes, nose, mouth, bandana, belly, tail) are
overlaid. States only change eyes / mouth / props / one paw, so the body stays put.
Run:  python3 docs/make_airou.py && claude-airou validate Sources/ClaudeAirou/Resources/pets/airou-felyne.json
"""
import json
import pathlib

W = H = 20

PALETTE = {
    "k": "#4A3428",  # outline (dark brown)
    "c": "#F4EAD5",  # cream fur
    "t": "#E2CDA4",  # tan shading / belly
    "p": "#F2A6B6",  # pink ear insides / nose
    "e": "#1F1611",  # eyes
    "w": "#FFFFFF",  # eye highlight
    "r": "#D9483B",  # bandana
    "d": "#A8332A",  # bandana shade
    "y": "#F5C518",  # sparkle / question mark
    "b": "#4C9AFF",  # thought dots / sweat
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

# ---------------------------------------------------------------- body

def body_mask(raised_paw=None):
    m = blank()
    ellipse(m, 9.5, 7.6, 7.6, 5.9)          # big round head (x 2..17, y 2..13)
    triangle(m, (3.0, -0.2), (0.4, 6.4), (8.6, 3.2))     # left ear: big upright triangle
    triangle(m, (16.0, -0.2), (18.6, 6.4), (10.4, 3.2))  # right ear
    rect(m, 6, 13, 13, 17)                  # small chubby torso
    rect(m, 4, 14, 5, 16)                   # left paw
    rect(m, 14, 14, 15, 16)                 # right paw
    rect(m, 6, 18, 8, 19)                   # left foot
    rect(m, 11, 18, 13, 19)                 # right foot
    return m

def base(tail=0, raised_paw=None):
    g = render(body_mask())
    # ears: pink inner triangles (only where the ear is not outline)
    inner = blank()
    triangle(inner, (3.2, 1.6), (1.9, 5.0), (6.4, 3.4))
    triangle(inner, (15.8, 1.6), (17.1, 5.0), (12.6, 3.4))
    for y in range(H):
        for x in range(W):
            if inner[y][x] and g[y][x] == "c":
                g[y][x] = "p"
    # big shiny oval eyes: 4x4, rounded corners, two highlights
    for x0 in (4, 12):
        for x in range(x0, x0 + 4):
            for y in range(6, 10):
                put(g, x, y, "e")
        for (x, y) in ((x0, 6), (x0 + 3, 6), (x0, 9), (x0 + 3, 9)):
            put(g, x, y, "c")
        put(g, x0 + 1, 6, "w"); put(g, x0 + 2, 8, "w")
    # nose (pink triangle) + ω mouth
    put(g, 9, 10, "p"); put(g, 10, 10, "p"); put(g, 9, 11, "p")
    put(g, 7, 11, "k"); put(g, 11, 11, "k"); put(g, 8, 12, "k"); put(g, 10, 12, "k")
    # whiskers poking out of the cheeks
    for (x, y) in ((1, 8), (2, 8), (1, 10), (17, 8), (18, 8), (18, 10)):
        put(g, x, y, "k")
    # thin red bandana at the neck, knot on the right
    for x in range(6, 14):
        put(g, x, 13, "r"); put(g, x, 14, "d" if x in (6, 13) else "r")
    put(g, 14, 13, "r"); put(g, 15, 13, "k"); put(g, 15, 14, "r"); put(g, 16, 14, "k")
    # belly patch
    for x in range(8, 12):
        for y in range(15, 17):
            put(g, x, y, "t")
    # tail: dark line curling up on the right, fluffy 2x2 tip; two poses
    if tail == 0:
        for (x, y) in ((14, 17), (15, 17), (16, 16), (17, 15), (17, 14)):
            put(g, x, y, "k")
        for (x, y) in ((16, 12), (17, 12), (16, 13), (17, 13)):
            put(g, x, y, "k")
        put(g, 17, 12, "t")
    else:
        for (x, y) in ((14, 17), (15, 17), (16, 16), (17, 15), (18, 14)):
            put(g, x, y, "k")
        for (x, y) in ((17, 12), (18, 12), (17, 13), (18, 13)):
            put(g, x, y, "k")
        put(g, 18, 12, "t")
    # raised paw (hello): 4x4 fist beside the head with a 2-wide arm down to the shoulder
    if raised_paw is not None:
        px, py = raised_paw
        for dx in range(4):
            for dy in range(4):
                put(g, px + dx, py + dy, "k")
        for dx in (1, 2):
            for dy in (1, 2):
                put(g, px + dx, py + dy, "c")
        for y in range(py + 4, 14):
            put(g, px, y, "c"); put(g, px + 1, y, "c"); put(g, px + 2, y, "k")
        for (x, y) in ((14, 14), (15, 14), (14, 15), (15, 15), (14, 16), (15, 16)):
            put(g, x, y, "c")  # right paw folded away while raised
        put(g, 15, 14, "r"); put(g, 16, 14, "k")
    return g

EYE_XS = (4, 5, 6, 7, 12, 13, 14, 15)

def clear_eyes(g):
    for x in EYE_XS:
        for y in (5, 6, 7, 8, 9):
            if g[y][x] in ("e", "w"):
                g[y][x] = "c"

def big_eyes(g, dy=0, dx=0):
    """Standard shiny eyes shifted by (dx, dy)."""
    for x0 in (4 + dx, 12 + dx):
        for x in range(x0, x0 + 4):
            for y in range(6 + dy, 10 + dy):
                put(g, x, y, "e")
        for (x, y) in ((x0, 6 + dy), (x0 + 3, 6 + dy), (x0, 9 + dy), (x0 + 3, 9 + dy)):
            put(g, x, y, "c")
        put(g, x0 + 1, 6 + dy, "w"); put(g, x0 + 2, 8 + dy, "w")

def rows(g):
    return ["".join(r) for r in g]

frames = {}

# idle: tail sways, one blink
i0 = base(tail=0); i1 = base(tail=1); i2 = base(tail=0)
blink = base(tail=0); clear_eyes(blink)
for x0 in (4, 12):
    put(blink, x0, 7, "e"); put(blink, x0 + 1, 8, "e"); put(blink, x0 + 2, 8, "e"); put(blink, x0 + 3, 7, "e")
frames["idle"] = [rows(i0), rows(i1), rows(i2), rows(blink)]

# thinking: eyes look up-right (pupils shift), blue dots top-right
def thinking(dots):
    g = base(tail=0); clear_eyes(g)
    big_eyes(g, dy=-1, dx=1)
    for (x, y) in dots:
        put(g, x, y, "b")
    return g
frames["thinking"] = [rows(thinking([(19, 2)])), rows(thinking([(18, 3), (19, 1), (19, 0)]))]

# working: focused (eyes 3x2 lower + brows), left arm nudges
def working(arm_up):
    g = base(tail=0); clear_eyes(g)
    for x0 in (4, 12):
        for x in range(x0, x0 + 4):
            for y in (8, 9):
                put(g, x, y, "e")
        put(g, x0, 9, "c"); put(g, x0 + 3, 9, "c")
        put(g, x0 + 1, 8, "w")
    put(g, 4, 6, "k"); put(g, 5, 7, "k"); put(g, 15, 6, "k"); put(g, 14, 7, "k")  # slanted brows
    if arm_up:
        for y in (14, 15, 16): put(g, 4, y, "c" if y != 14 else "c"); put(g, 5, y, "c")
        put(g, 4, 13, "k"); put(g, 5, 13, "k"); put(g, 4, 14, "k"); put(g, 5, 14, "c"); put(g, 4, 15, "k"); put(g, 5, 15, "c"); put(g, 4, 16, "k"); put(g, 5, 16, "k")  # paw one row up
    return g
frames["working"] = [rows(working(False)), rows(working(True))]

# waiting_approval: wide eyes (3x4) + red "!" floating top-right (bar, gap, dot)
def waiting(shift):
    g = base(tail=0); clear_eyes(g)
    for x0 in (4, 12):
        for x in range(x0, x0 + 4):
            for y in range(5, 10):
                put(g, x, y, "e")
        for (x, y) in ((x0, 5), (x0 + 3, 5), (x0, 9), (x0 + 3, 9)):
            put(g, x, y, "c")
        put(g, x0 + 1, 5, "w"); put(g, x0 + 2, 5, "w"); put(g, x0 + 2, 8, "w")
    put(g, 7, 11, "c"); put(g, 11, 11, "c"); put(g, 8, 12, "c"); put(g, 10, 12, "c")
    put(g, 9, 12, "e"); put(g, 10, 12, "e")  # small "o" mouth
    for y in range(0 + shift, 3 + shift):
        put(g, 18, y, "r"); put(g, 19, y, "r")
    put(g, 18, 4 + shift, "r"); put(g, 19, 4 + shift, "r")
    return g
frames["waiting_approval"] = [rows(waiting(0)), rows(waiting(1))]

# needs_input: half-lidded + yellow "?"
def needs_input(dy):
    g = base(tail=0); clear_eyes(g)
    for x0 in (4, 12):
        for x in range(x0, x0 + 4):
            for y in (8, 9):
                put(g, x, y, "e")
        put(g, x0, 9, "c"); put(g, x0 + 3, 9, "c")
        put(g, x0 + 1, 8, "w")
        for x in range(x0, x0 + 4):
            put(g, x, 7, "k")  # lid
    q = ["yyy", "..y", ".yy", "...", ".y."]
    for r, line in enumerate(q):
        for c, ch in enumerate(line):
            if ch == "y":
                put(g, 17 + c, r + dy, "y")
    return g
frames["needs_input"] = [rows(needs_input(0)), rows(needs_input(1))]

# done: ^^ eyes + big smile + sparkles
def done(alt):
    g = base(tail=1 if alt else 0); clear_eyes(g)
    for x0 in (4, 12):
        put(g, x0 + 1, 7, "e"); put(g, x0 + 2, 7, "e"); put(g, x0, 8, "e"); put(g, x0 + 3, 8, "e")
    put(g, 7, 11, "k"); put(g, 11, 11, "k")
    for x in range(8, 11):
        put(g, x, 12, "k")   # wide smile under the corners
    for (x, y) in ([(1, 3), (18, 7), (2, 12)] if not alt else [(0, 6), (19, 3), (1, 10)]):
        put(g, x, y, "y")
    return g
frames["done"] = [rows(done(False)), rows(done(True))]

# error: x x eyes + sweat drop
def error(drop_dy):
    g = base(tail=0); clear_eyes(g)
    for x0 in (4, 12):
        for (dx, dy) in ((0, 0), (3, 0), (1, 1), (2, 1), (1, 2), (2, 2), (0, 3), (3, 3)):
            put(g, x0 + dx, 6 + dy, "e")
    put(g, 7, 11, "c"); put(g, 11, 11, "c"); put(g, 8, 12, "c"); put(g, 10, 12, "c")
    put(g, 7, 12, "k"); put(g, 11, 12, "k"); put(g, 8, 11, "k"); put(g, 9, 11, "k"); put(g, 10, 11, "k")  # frown
    put(g, 18, 6 + drop_dy, "b"); put(g, 18, 7 + drop_dy, "b")
    return g
frames["error"] = [rows(error(0)), rows(error(2))]

# hello: raised right paw waving (fist held out beside the head, clear of its outline)
frames["hello"] = [rows(base(tail=0, raised_paw=(16, 8))), rows(base(tail=1, raised_paw=(16, 6)))]

pet = {
    "id": "airou-felyne",
    "name": "Airou",
    "species": "felyne",
    "description": "A Felyne-style hunting companion in a red bandana. Carves your builds for materials, nya.",
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
