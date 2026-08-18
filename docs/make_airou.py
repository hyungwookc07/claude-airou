"""Generate the Airou pet — a Monster-Hunter-Felyne-inspired chibi cat — for claude-airou.

Felyne cues this leans on: cream/white fur with darker (tan) tips on the ears, paws, feet and
tail; an oversized round head with puffy cheeks; big pointed ears with pink insides; large round
eyes; a small pink triangular nose over an "ω" mouth; whiskers; a long thin tail ending in a tuft.

Silhouette pieces are painted into a mask, the outline is derived automatically (mask pixels
touching transparency), then features are overlaid. States only change eyes / mouth / props /
one paw, so the body stays put.
Run:  python3 docs/make_airou.py && claude-airou validate Sources/ClaudeAirou/Resources/pets/airou-felyne.json
"""
import json
import pathlib

W = H = 24

PALETTE = {
    "k": "#4A3428",  # outline (dark brown)
    "c": "#F7F2E6",  # cream-white fur
    "t": "#D6B283",  # tan tips (ears, paws, feet, tail tuft) and belly
    "p": "#F0A3B4",  # pink ear insides / nose
    "e": "#1F1611",  # eyes
    "w": "#FFFFFF",  # eye highlight
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

# ---------------------------------------------------------------- body

LEFT_EAR = ((4.0, -0.3), (0.6, 7.2), (9.6, 4.2))
RIGHT_EAR = ((19.0, -0.3), (22.4, 7.2), (13.4, 4.2))

def body_mask():
    m = blank()
    ellipse(m, 11.5, 9.6, 9.1, 7.0)         # big round head (x 3..20, y 3..16)
    ellipse(m, 4.6, 12.2, 2.4, 2.2)         # left puffy cheek
    ellipse(m, 18.4, 12.2, 2.4, 2.2)        # right puffy cheek
    triangle(m, *LEFT_EAR)                  # big pointed ears
    triangle(m, *RIGHT_EAR)
    rect(m, 7, 16, 16, 20)                  # chubby torso
    rect(m, 5, 17, 6, 19)                   # left paw
    rect(m, 17, 17, 18, 19)                 # right paw
    rect(m, 7, 21, 9, 23)                   # left foot
    rect(m, 14, 21, 16, 23)                 # right foot
    return m

def base(tail=0, raised_paw=None):
    g = render(body_mask())

    # ears: pink inner triangles, tan tips
    inner = blank()
    triangle(inner, (4.2, 2.0), (2.4, 6.0), (7.4, 4.4))
    triangle(inner, (18.8, 2.0), (20.6, 6.0), (15.6, 4.4))
    paint_inside(g, inner, "p")
    tips = blank()
    triangle(tips, LEFT_EAR[0], (2.6, 3.0), (6.4, 1.9))
    triangle(tips, RIGHT_EAR[0], (20.4, 3.0), (16.6, 1.9))
    paint_inside(g, tips, "t", only=("c", "p"))

    # big round eyes (5x5, rounded corners) with two highlights
    for x0 in (5, 14):
        for x in range(x0, x0 + 5):
            for y in range(8, 13):
                put(g, x, y, "e")
        for (x, y) in ((x0, 8), (x0 + 4, 8), (x0, 12), (x0 + 4, 12)):
            put(g, x, y, "c")
        put(g, x0 + 1, 9, "w"); put(g, x0 + 2, 9, "w"); put(g, x0 + 3, 11, "w")

    # pink triangular nose over an "ω" mouth
    put(g, 11, 13, "p"); put(g, 12, 13, "p"); put(g, 11, 14, "p"); put(g, 12, 14, "p")
    for (x, y) in ((9, 15), (11, 15), (13, 15), (10, 16), (12, 16)):
        put(g, x, y, "k")

    # whiskers, two per side, poking past the cheeks
    for (x, y) in ((1, 11), (2, 11), (1, 13), (2, 13), (21, 11), (22, 11), (21, 13), (22, 13)):
        put(g, x, y, "k")

    # tan paw tips / feet tips (Felyne "socks")
    for (x, y) in ((5, 19), (6, 19), (17, 19), (18, 19)):
        put(g, x, y, "t")
    for x in (7, 8, 9, 14, 15, 16):
        put(g, x, 22, "t")

    # belly
    for x in range(10, 14):
        for y in range(18, 21):
            put(g, x, y, "t")

    # long thin tail curling up on the right, ending in a fluffy tan tuft; two poses
    if tail == 0:
        for (x, y) in ((17, 21), (18, 22), (19, 21), (20, 20), (21, 19), (21, 18)):
            put(g, x, y, "k")
        for dx in range(3):
            for dy in range(3):
                put(g, 19 + dx, 15 + dy, "k")
        put(g, 20, 16, "t")
    else:
        for (x, y) in ((17, 21), (18, 22), (19, 21), (20, 20), (21, 19), (22, 18)):
            put(g, x, y, "k")
        for dx in range(3):
            for dy in range(3):
                put(g, 20 + dx, 15 + dy, "k")
        put(g, 21, 16, "t")

    # raised paw (hello): 4x4 fist beside the head with a 2-wide arm down to the shoulder
    if raised_paw is not None:
        px, py = raised_paw
        for dx in range(4):
            for dy in range(4):
                put(g, px + dx, py + dy, "k")
        for dx in (1, 2):
            for dy in (1, 2):
                put(g, px + dx, py + dy, "c")
        put(g, px + 1, py + 2, "t"); put(g, px + 2, py + 2, "t")   # tan paw tip
        for y in range(py + 4, 17):
            put(g, px, y, "c"); put(g, px + 1, y, "c"); put(g, px + 2, y, "k")
        for (x, y) in ((17, 17), (18, 17), (17, 18), (18, 18), (17, 19), (18, 19)):
            put(g, x, y, "c")   # right paw folded away while raised
        put(g, 17, 20, "k"); put(g, 18, 20, "k")
    return g

EYE_ROWS = (7, 8, 9, 10, 11, 12)

def clear_eyes(g):
    for x0 in (5, 14):
        for x in range(x0, x0 + 5):
            for y in EYE_ROWS:
                if g[y][x] in ("e", "w"):
                    g[y][x] = "c"

def big_eyes(g, dx=0, dy=0, tall=5):
    for x0 in (5 + dx, 14 + dx):
        y0 = 8 + dy
        for x in range(x0, x0 + 5):
            for y in range(y0, y0 + tall):
                put(g, x, y, "e")
        for (x, y) in ((x0, y0), (x0 + 4, y0), (x0, y0 + tall - 1), (x0 + 4, y0 + tall - 1)):
            put(g, x, y, "c")
        put(g, x0 + 1, y0 + 1, "w"); put(g, x0 + 2, y0 + 1, "w"); put(g, x0 + 3, y0 + tall - 2, "w")

def clear_mouth(g):
    for (x, y) in ((9, 15), (11, 15), (13, 15), (10, 16), (12, 16)):
        put(g, x, y, "c")

def rows(g):
    return ["".join(r) for r in g]

frames = {}

# idle: tail sways, one blink
i0 = base(tail=0); i1 = base(tail=1); i2 = base(tail=0)
blink = base(tail=0); clear_eyes(blink)
for x0 in (5, 14):
    put(blink, x0, 9, "e"); put(blink, x0 + 1, 10, "e"); put(blink, x0 + 2, 10, "e"); put(blink, x0 + 3, 10, "e"); put(blink, x0 + 4, 9, "e")
frames["idle"] = [rows(i0), rows(i1), rows(i2), rows(blink)]

# thinking: eyes drift up-right + blue dots
def thinking(dots):
    g = base(tail=0); clear_eyes(g)
    big_eyes(g, dx=1, dy=-1)
    for (x, y) in dots:
        put(g, x, y, "b")
    return g
frames["thinking"] = [rows(thinking([(23, 3)])), rows(thinking([(22, 4), (23, 2), (23, 0)]))]

# working: focused — lower half of the eyes + slanted brows, left paw nudges up
def working(paw_up):
    g = base(tail=0); clear_eyes(g)
    for x0 in (5, 14):
        for x in range(x0, x0 + 5):
            for y in (10, 11, 12):
                put(g, x, y, "e")
        put(g, x0, 12, "c"); put(g, x0 + 4, 12, "c")
        put(g, x0 + 1, 10, "w"); put(g, x0 + 2, 10, "w")
    put(g, 5, 8, "k"); put(g, 6, 9, "k"); put(g, 18, 8, "k"); put(g, 17, 9, "k")
    if paw_up:
        for y in (17, 18, 19):
            put(g, 5, y, "c"); put(g, 6, y, "c")
        put(g, 5, 16, "k"); put(g, 6, 16, "k"); put(g, 5, 17, "k"); put(g, 5, 18, "t"); put(g, 6, 18, "t"); put(g, 5, 19, "k"); put(g, 6, 19, "k")
    return g
frames["working"] = [rows(working(False)), rows(working(True))]

# waiting_approval: even bigger eyes, small "o" mouth, red "!" floating clear of the right ear
def waiting(shift):
    g = base(tail=0); clear_eyes(g); clear_mouth(g)
    big_eyes(g, dy=-1, tall=6)
    put(g, 11, 15, "e"); put(g, 12, 15, "e"); put(g, 11, 16, "e"); put(g, 12, 16, "e")
    for y in range(0 + shift, 3 + shift):
        put(g, 22, y, "r"); put(g, 23, y, "r")
    put(g, 22, 4 + shift, "r"); put(g, 23, 4 + shift, "r")
    return g
frames["waiting_approval"] = [rows(waiting(0)), rows(waiting(1))]

# needs_input: half-lidded + yellow "?"
def needs_input(dy):
    g = base(tail=0); clear_eyes(g)
    for x0 in (5, 14):
        for x in range(x0, x0 + 5):
            for y in (10, 11, 12):
                put(g, x, y, "e")
        put(g, x0, 12, "c"); put(g, x0 + 4, 12, "c")
        put(g, x0 + 1, 10, "w"); put(g, x0 + 2, 10, "w")
        for x in range(x0, x0 + 5):
            put(g, x, 9, "k")  # lid
    q = ["yyy", "..y", ".yy", "...", ".y."]
    for r, line in enumerate(q):
        for c, ch in enumerate(line):
            if ch == "y":
                put(g, 21 + c, r + dy, "y")
    return g
frames["needs_input"] = [rows(needs_input(0)), rows(needs_input(1))]

# done: ^^ eyes + big open smile + sparkles
def done(alt):
    g = base(tail=1 if alt else 0); clear_eyes(g); clear_mouth(g)
    for x0 in (5, 14):
        put(g, x0 + 2, 9, "e"); put(g, x0 + 1, 10, "e"); put(g, x0 + 3, 10, "e"); put(g, x0, 11, "e"); put(g, x0 + 4, 11, "e")
    put(g, 9, 15, "k"); put(g, 14, 15, "k")
    for x in range(10, 14):
        put(g, x, 16, "k")
    for (x, y) in ([(1, 5), (22, 9), (2, 15)] if not alt else [(0, 8), (23, 6), (1, 12)]):
        put(g, x, y, "y")
    return g
frames["done"] = [rows(done(False)), rows(done(True))]

# error: x x eyes + frown + sweat drop
def error(drop_dy):
    g = base(tail=0); clear_eyes(g); clear_mouth(g)
    for x0 in (5, 14):
        for (dx, dy) in ((0, 0), (4, 0), (1, 1), (3, 1), (2, 2), (1, 3), (3, 3), (0, 4), (4, 4)):
            put(g, x0 + dx, 8 + dy, "e")
    put(g, 9, 16, "k"); put(g, 14, 16, "k")
    for x in range(10, 14):
        put(g, x, 15, "k")
    put(g, 22, 8 + drop_dy, "b"); put(g, 22, 9 + drop_dy, "b")
    return g
frames["error"] = [rows(error(0)), rows(error(2))]

# hello: raised right paw waving beside the head
frames["hello"] = [rows(base(tail=0, raised_paw=(20, 9))), rows(base(tail=1, raised_paw=(20, 7)))]

pet = {
    "id": "airou-felyne",
    "name": "Airou",
    "species": "felyne",
    "description": "A Felyne-style hunting companion: cream fur, tan-tipped ears and paws, big round eyes. Carves your builds for materials, nya.",
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
