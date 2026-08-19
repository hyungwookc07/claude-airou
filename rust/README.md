# claude-airou (Rust)

The cross-platform rewrite of claude-airou — macOS first, Windows and Linux next.
**End goal: this crate fully replaces the Swift app.** Until it reaches overlay parity,
both implementations live side by side and share every on-disk format, so you can mix
them freely (e.g. Swift overlay + Rust hook/MCP, or the other way around).

한국어로 코드를 따라 읽고 싶다면 → [LEARNING.ko.md](LEARNING.ko.md) (이 코드베이스를 교재로 쓰는 러스트 학습 가이드).

## Status

| Area | State |
|---|---|
| Core (`hook`, `mcp`, `statusline`, installers, CLI: simulate/pets/validate/render/preview/status) | Ported 1:1 from Swift, unit tests + end-to-end smoke run on Linux CI-style checks |
| Overlay (floating pet, tray menu) | macOS v0.2 stage 2: at parity with the Swift overlay on real hardware — transparent CALayer-presented canvas, system-font text with Hangul (incl. SF's `trak` letter spacing), **session fan-out row** (primary pet in the middle, side pets with their own gauge/label/badge, unfold/fold springs, pinning by click or menu), Swift `RowLayout` geometry, the full tray/right-click menu, `snapshot` / `click` test commands, hop/shake/heart/pulse/spin animations, AppKit-convention window position shared with Swift. Emoji in bubbles and SIGTERM cleanup are the known gaps (see below) |
| Windows / Linux overlay | Not started (the rest of the binary already runs there) |

Everything the binaries write — `~/.claude-airou/state/*.json`, `*.usage.json`, `config.json`,
pet packs, `~/.claude/settings.json` edits, `claude_desktop_config.json` edits — is
byte-compatible with the Swift app. The built-in pets are `include_str!`-ed from
`../Sources/ClaudeAirou/Resources/pets/`, so there is exactly one copy of the art in the repo.

## Build & try (macOS)

```bash
cd rust
cargo build --release          # → target/release/claude-airou
./target/release/claude-airou pets
./target/release/claude-airou simulate demo   # watch it with the (Swift or Rust) overlay running
./target/release/claude-airou run             # the Rust overlay itself (v0.2)
./target/release/claude-airou snapshot --out ~/Desktop/airou.png   # ask the running overlay for a PNG
./target/release/claude-airou click primary   # scripted click (expand / collapse / pet)
```

The CLI surface is identical to the Swift binary (`claude-airou help`). To point Claude Code
hooks / the status line / the Claude desktop app at the Rust binary instead of the Swift one,
run its own installers (they edit the same config files, with backups):

```bash
./target/release/claude-airou install-hooks
./target/release/claude-airou install-statusline
./target/release/claude-airou install-mcp
```

## Overlay: what is done, what remains

Done in v0.2 (stage 1 + stage 2, verified on a real Mac next to the Swift overlay with
three sessions, one waiting for approval — screenshots matched to the point):

- **True per-pixel transparency.** `overlay/draw.rs` is a premultiplied-RGBA software
  compositor (anti-aliased capsules, circles, lines, sprite blits, text, scaled/faded
  canvas blits, PNG export) and `overlay/present_macos.rs` hands the buffer to the winit
  window's `CALayer` as a `CGImage`; the NSWindow is non-opaque with a clear background
  and no shadow. Only drawn pixels are visible — no card.
- **Real text.** `overlay/text.rs` rasterises with `ab_glyph` through a per-style font
  chain (`platform_font_files`, the one platform-specific corner): SF (`opsz` Text cut,
  `wght` medium/semibold/bold), SF Rounded for labels, Helvetica, and Apple SD Gothic Neo
  for Hangul/CJK, with per-glyph fallback, a glyph cache, the face's AAT `trak`
  tracking (SF's generous small-size letter spacing — without it labels were ~10 %
  narrower than AppKit's) and tabular digits for the gauge (`.monospacedDigit()`).
- **Session fan-out row** (`overlay/row_layout.rs` = Swift `RowLayout`, `overlay/logic.rs`
  = `PetViewModel`): one card per session when fanned out (click the pet, or "Show all
  sessions side by side"), primary in the middle at full size, side pets at 70 % scale
  (min 2 px) and 92 % opacity with compact gauge + label + badge, others alternating
  right/left by recency, `+N` label + red attention dot when collapsed, accent-coloured
  primary capsule when expanded. Clicking a side pet pins it (also from the Sessions
  menu; "Automatic" un-pins); clicking the primary or a gap folds the row back
  (0.22 s ease-in) and collapses; the panel is resized around the primary pet
  (`overlay/placement.rs`) so it never moves on screen, and the bubble stays centred
  over it (the row reserves the bubble's measured width). Motion is FLIP in screen
  space (`overlay/animation.rs`, spring 0.34 s / bounce 0.18).
- **State animations** (Swift `PetView`): done/hello hop (`[0,-14,0,-7,0]` × 0.16 s),
  error shake (`[0,-5,5,-4,4,0]` × 0.06 s), floating heart on click (1.1 s), speech
  bubble fade+scale entrance (0.16 s), badge pop-in (0.25 s), pulsing clock / `?`
  (1 s cycle), spinning gear (2.4 s per turn). Short animations redraw at 60 Hz, the
  continuous badge effects at the 10 Hz tick.
- **Window position** is stored exactly like Swift: `windowOriginX` = where the collapsed
  panel's left edge would be, `windowOriginY` = AppKit bottom-left screen y (points, main
  display origin) — read and written through the NSWindow frame, never winit's flipped
  coordinates. Placement/nudge/default-corner rules are the `OverlayPanel` ones
  (`NSScreen.visibleFrame`, 25 % probe, 24 pt margin), multi-monitor included.
- **Menu** (tray + right-click / control-click on the pet, `overlay/tray.rs`), Swift
  order and wording: header + usage line, Sessions (N), Gauge (+ "Feed from Claude Code
  status line…"), fan-out toggle, Pet (+ Reload, Open pets folder…), Size, Hide bubbles /
  Click-through / Hide pet / Reset position, Install Claude Code hooks…, Open hook log,
  Quit ⌘Q. Installer results show in an `NSAlert` like Swift.
- **Test commands**: `claude-airou snapshot [--out FILE.png]` (the overlay renders its own
  content to `snapshot.png` — no screen-recording permission) and
  `claude-airou click [primary|X]`, via the same `snapshot.request` / `click.request`
  files Swift uses; answered every 0.4 s. Clicks are logged to `overlay.log` in the Swift
  format (`click x=… cards: … -> expand|collapse|pin …|pet`).
- Light/dark appearance, the accent colour and `windowBackgroundColor` are read from
  AppKit (re-checked periodically).
- `make autostart` / `make no-autostart` in `rust/Makefile` write the same LaunchAgent as
  the Swift Makefile (`dev.claude-airou.overlay`), pointing at `~/.local/bin/claude-airou`.

Still missing / different:

- Emoji in bubbles: Apple Color Emoji is a bitmap font `ab_glyph` cannot outline, so
  emoji currently advance as blank space.
- Badge icons are hand-drawn shapes rather than SF Symbols (same sizes/colours; the gear
  and wave are approximations).
- Session cleanup on SIGTERM is best-effort (no signal-handling crate); the overlay's
  stale-state decay covers the gap. Killing the overlay with SIGKILL leaves the pid lock
  behind for up to a day (delete `overlay.lock` or wait) — the Swift app uses `flock`.
- The bubble's disappearance is immediate (Swift fades it out over 0.16 s) and the
  label capsule width snaps when the badge appears (Swift animates it).

## Known deviations from Swift

Deliberate, all edge-case-only (an adversarial review pass hunted for more; anything not
listed here is meant to match 1:1 — file a bug if it doesn't):

- **Files are format-compatible, not byte-identical.** Each binary decodes the other's
  files; key order and float formatting differ (Swift sorts keys and prints `1755500000`,
  serde_json keeps declaration order and prints `1755500000.0`).
- **"1 character" = 1 Unicode scalar, not 1 grapheme.** Bubble truncation, pet
  validation and the row layout's label-width estimate count scalars (`char`), Swift
  counts grapheme clusters — ZWJ emoji and NFD-decomposed text diverge (never a crash).
  Palette keys are single-scalar in practice.
- **`windowOriginX` is anchored to the bubble-less collapsed layout.** Swift subtracts
  the collapsed layout *including the current bubble width*, so quitting Swift while a
  wide bubble shows shifts the pet on the next launch; Rust stores the bubble-less
  value (identical whenever no bubble is showing, which is when both apps normally
  save). Both apps read either value the same way.
- **Whitespace trimming** uses Rust `str::trim` (includes newlines) where Swift's pet
  validation uses spaces+tab only; verdicts differ only for pathological inputs like
  `"#112233\n"`.
- **Error wording** for unreadable/malformed pet files comes from serde/std instead of
  Cocoa's `localizedDescription`; validation problem strings themselves match 1:1.
- `CLAUDE_AIROU_HOME=~user/...` (the *other-user* tilde form) is not expanded; `~/` is.
- Rust-only safety guard: absurdly large render sheets fail with a clean error before any
  file is written instead of aborting on allocation.

## Roadmap to full replacement

1. **v0.1** — core parity + first overlay (done).
2. **v0.2 — overlay parity.** Stage 1 (done): true transparency, system-font text
   (Korean bubbles). Stage 2 (done): session fan-out + pinning, state animations,
   `snapshot`/`click` commands, Swift-convention window position, full menu, autostart
   Makefile targets. Exit criterion: a week of daily use without missing the Swift
   overlay (open: emoji in bubbles).
3. **v1.0 — replace Swift.** Move `Sources/ClaudeAirou/Resources/pets/` to a neutral
   `pets/` directory (update the `include_str!` paths and the Swift package resource list —
   or delete the Swift package in the same change), point the Makefile at cargo, delete
   `Sources/`. State files, configs and installers need no migration — they were shared
   all along.
4. **v1.1 — Windows overlay** (tray + layered window), then Linux (X11 first; Wayland
   always-on-top/click-through is compositor-dependent).

## Layout

```
src/
  main.rs, cli.rs        entry + arg parsing/dispatch (mirrors Swift CommandLineInterface)
  model.rs               PetState, SessionSnapshot, usage, AppConfig  ← on-disk contract
  state_store.rs         ~/.claude-airou/state/*.json (atomic writes, stale pruning)
  paths.rs, logging.rs   locations + truncating logs
  hook.rs, hook_mapper.rs    `claude-airou hook` (event → state mapping, merge policy)
  mcp.rs, mcp_tools.rs       `claude-airou mcp` (Claude chat integration)
  statusline.rs          usage recording + passthrough, transcript estimator
  install.rs             hooks / statusline / MCP installers (backup + idempotent)
  pets.rs, render.rs     pet packs, validation, PNG/ASCII rendering
  cli_commands.rs        simulate / pets / validate / render / preview / status / snapshot / click
  overlay/               macOS overlay — cfg(target_os):
    window.rs              event loop, panel geometry, painting (cards, bubble, gauge, label, badge), input
    row_layout.rs          Swift RowLayout port (card widths/positions, bubble room) — pure
    logic.rs               Swift PetViewModel port (focus rule, pinning, fan-out, clicks, triggers) — pure
    animation.rs           timing curves (spring, phase animators, heart, pulse, gear, FLIP) — pure
    placement.rs           AppKit-coordinate panel placement / nudge / resize-around-primary — pure
    draw.rs                RGBA compositor (+ scaled/faded canvas blits, heart, PNG export)
    text.rs                ab_glyph font chain, SF `trak` tracking, tabular digits
    present_macos.rs       CALayer presenter, NSWindow/NSScreen/NSColor/NSAlert/context-menu bridge
    tray.rs                menu model + muda menu (tray and right-click), lock.rs (pid lock)
```

Tests: `cargo test` (300 unit tests, run everywhere; the overlay's pure modules — row
layout, focus/click logic, animation curves, placement, click/snapshot request parsing —
are covered without a window; a few font tests need macOS), plus `python3 integration_test.py` — a 40-check battery that drives the real
binary end to end (hook lifecycle incl. the approval merge policy, MCP conversation with a
real hatched PNG, installers against fixture files, Swift-file interop, robustness) inside
a throwaway sandbox. Cross-check without a Mac:
`rustup target add aarch64-apple-darwin && cargo check --target aarch64-apple-darwin`.
