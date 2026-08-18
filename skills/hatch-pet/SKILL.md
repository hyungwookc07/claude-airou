---
name: hatch-pet
description: Hatch a new custom pet for claude-airou (the Codex-style desktop pet for Claude Code). Use when the user asks to create, design, hatch, or edit a claude-airou pet/companion/creature (e.g. "/hatch-pet a sleepy axolotl", "make me a pet that looks like a goblin", "change my pet's colors"). Produces a validated pixel-art JSON in ~/.claude-airou/pets/ and shows the rendered sprite sheet.
---

# Hatch a claude-airou pet

You are designing a pixel-art companion that lives in a floating overlay next to the user's
Claude Code sessions. The overlay swaps sprites as Claude works: `idle`, `thinking`, `working`,
`waiting_approval`, `needs_input`, `done`, `error`, `hello`. The pet is a JSON file; the
`claude-airou` binary validates and renders it.

## Workflow

1. **Concept.** Take the concept from the arguments. If none, ask one short question:
   what creature/object, and any colour or personality wishes. Do not over-ask.
2. **Pick an id and name.** `id` is kebab-case, unique, e.g. `nori-axolotl`. `name` is the
   display name (e.g. `Nori`). `species` is a short noun (`axolotl`).
3. **Design the sprite** following the format and design rules below. Start from the
   `idle` frame, then derive every other state by editing only the parts that must change
   (eyes, mouth, small props). Keep the body identical across states so the animation does not
   jump.
4. **Write the file** to `~/.claude-airou/pets/<id>.json` (create the folder if needed).
5. **Validate**: `claude-airou validate ~/.claude-airou/pets/<id>.json`. Fix every error
   (mismatched row widths are the usual culprit; count characters). Warnings about missing
   states are fine only if the fallback is acceptable.
6. **Look at it**: `claude-airou render <id> --out /tmp/claude-airou-render-<id> --scale 10` and
   open `sheet.png` with the Read tool. Rows are states in this order: hello, idle, thinking,
   working, waiting_approval, needs_input, done, error; columns are frames. Judge it honestly:
   is the silhouette readable? Are the eyes visible? Do the states differ? Iterate at least once.
7. **Hand over**: tell the user the pet is available in the menu bar 🐾 → Pet → `<name>`
   (or "Reload pets" if the overlay is already running), and show the sheet path.
   If `claude-airou` is not on PATH, use `~/.local/bin/claude-airou` or the repo's
   `.build/release/claude-airou`.

If asked to *edit* an existing pet, load the JSON, change only what was asked, and repeat
steps 5–7.

## File format

```json
{
  "id": "nori-axolotl",
  "name": "Nori",
  "species": "axolotl",
  "description": "A pink axolotl who is convinced every build will pass.",
  "author": "hatch-pet",
  "fps": 3,
  "palette": {
    "k": "#3a2a2a",
    "p": "#f6a7c1",
    "d": "#e0839f",
    "w": "#ffffff",
    "e": "#222222"
  },
  "phrases": {
    "pet": ["blub.", "Regeneration in progress.", "Was that a merge conflict?"]
  },
  "frames": {
    "idle": [ ["....", "...."], ["....", "...."] ],
    "thinking": [ ["....", "...."] ],
    "working": [ ["....", "...."] ],
    "waiting_approval": [ ["....", "...."] ],
    "needs_input": [ ["....", "...."] ],
    "done": [ ["....", "...."] ],
    "error": [ ["....", "...."] ],
    "hello": [ ["....", "...."] ]
  }
}
```

Rules enforced by `claude-airou validate`:

- `id`: letters, digits, `-`, `_` only. `name` non-empty.
- `palette`: keys are **single characters**; values are `#RRGGBB` or `#RRGGBBAA`.
  `.` and space are reserved for transparent and must not be palette keys.
- `frames.idle` is required. Each state maps to an **array of frames**; each frame is an
  **array of row strings**; every row in every frame of every state has the **same width**, and
  every frame has the **same number of rows** (one grid size per pet, min 4×4, max 64×64).
- Every non-transparent character in a frame must exist in the palette.
- Unknown state keys are ignored with a warning.
- `fps` is optional (default 3, clamped 0.5–12). `phrases.pet` is optional: random lines shown
  when the user clicks the pet.

Fallbacks when a state has no frames: `hello→done→idle`, `working→thinking→idle`,
`waiting_approval→needs_input→idle`, `needs_input→waiting_approval→idle`, everything else `→idle`.
The overlay also draws its own status badge (red clock for waiting_approval, green check for
done, etc.), so the sprite only needs an *expression* change, not an icon.

## Design rules that make pets look good

- **Grid**: 16×16 to 24×24 works best. The overlay scales pixels 3×/5×/7×.
- **Outline**: use one dark outline colour around the silhouette; it reads at small sizes.
- **Palette**: 4–8 colours. One base colour, one shade, one highlight/belly, eye white + pupil,
  plus 1–2 accent colours for state props (blue thought dots, yellow sparkle, red "!").
- **Idle**: 2–4 frames. Subtle motion only (blink on one frame, tail/ear/antenna sway).
  Do not move the whole body between frames.
- **thinking**: eyes look up/sideways, optional 1–3 blue dots above the head across frames.
- **working**: focused/narrowed eyes, small paw/hand/keyboard motion across 2 frames.
- **waiting_approval**: wide eyes / raised brows, optional red `!` above the head. This is the
  state the user must notice.
- **needs_input**: sleepy or expectant — half-closed eyes, maybe a `?` or `z`.
- **done**: happy closed eyes (`^ ^`), open smile, sparkles in the corners.
- **error**: `x x` eyes, small sweat drop or dizzy swirl.
- **hello**: a wave (one limb raised) or a bounce; can be omitted (falls back to `done`).
- Keep pupils 1×1 or 2×2 and eye whites 2×2 or wider — that is where character comes from.
- Leave the top row(s) mostly empty so props (dots, sparkles, `!`) have room.
- Never leave stray single pixels of body colour outside the outline.

## Sanity checklist before validating

- Count the characters of every row. Same width everywhere. Same row count everywhere.
- Every character used is in the palette (or is `.` / space).
- `frames.idle` has ≥ 1 frame.
- Transparent background is `.` — never a colour.
