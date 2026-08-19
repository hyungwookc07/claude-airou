# claude-airou (the Rust crate)

This crate *is* claude-airou: the hook, status line, MCP server, installers, CLI and the
macOS overlay. It started as a 1:1 port of the original Swift/AppKit app and replaced it in
v1.0 (the Swift sources were removed; they live on in git history at commit `3037817`,
which is what every "Swift" comparison below refers to). macOS overlay first, Windows and
Linux next — everything except the overlay already runs on all three.

한국어로 코드를 따라 읽고 싶다면 → [LEARNING.ko.md](LEARNING.ko.md) (이 코드베이스를 교재로 쓰는 러스트 학습 가이드).

## Status

| Area | State |
|---|---|
| Core (`hook`, `mcp`, `statusline`, installers, CLI: simulate/pets/validate/render/preview/status) | Done (ported 1:1 from Swift), unit tests + end-to-end battery, runs on macOS/Linux/Windows |
| Overlay (floating pet, tray menu) | macOS v1.0: at parity with the former Swift overlay on real hardware — transparent CALayer-presented canvas, system-font text with Hangul (incl. SF's `trak` letter spacing), **session fan-out row** (primary pet in the middle, side pets with their own gauge/label/badge, unfold/fold springs, pinning by click or menu), Swift `RowLayout` geometry, the full tray/right-click menu, `snapshot` / `click` test commands, hop/shake/heart/pulse/spin animations, AppKit-convention window position shared with Swift. Emoji in bubbles and SIGTERM cleanup are the known gaps (see below) |
| Windows / Linux overlay | Not started (the rest of the binary already runs there) |

Everything the binary writes — `~/.claude-airou/state/*.json`, `*.usage.json`, `config.json`,
pet packs, `~/.claude/settings.json` edits, `claude_desktop_config.json` edits — is
format-compatible with what the Swift app wrote, so upgrading needed no migration. The
built-in pets are `include_str!`-ed from the repo-root [`../pets/`](../pets/) directory.

## Build & try (macOS)

```bash
make build                     # from the repo root → rust/target/release/claude-airou
# or: cd rust && cargo build --release
./rust/target/release/claude-airou pets
./rust/target/release/claude-airou simulate demo   # watch it with the overlay running
./rust/target/release/claude-airou run             # the overlay itself
./rust/target/release/claude-airou snapshot --out ~/Desktop/airou.png   # ask the running overlay for a PNG
./rust/target/release/claude-airou click primary   # scripted click (expand / collapse / pet)
```

`claude-airou help` lists the CLI. The installers (`make hooks` / `make statusline` /
`make mcp`, or the `install-*` subcommands directly) edit the config files with backups.

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
- `make autostart` / `make no-autostart` (repo-root Makefile) write the LaunchAgent
  `dev.claude-airou.overlay`, pointing at `~/.local/bin/claude-airou`.

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

- **Files are format-compatible, not byte-identical.** Files written by the old Swift
  binary decode fine (and vice versa); key order and float formatting differ (Swift sorted
  keys and printed `1755500000`, serde_json keeps declaration order and prints `1755500000.0`).
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
3. **v1.0 — replace Swift** (done). The pet art moved to a neutral repo-root `pets/`
   directory, the root Makefile drives cargo, and `Sources/` + `Package.swift` were
   deleted. State files, configs and installers needed no migration — they were shared
   all along.
4. **v1.1 — Windows overlay** (tray + layered window), then Linux (X11 first; Wayland
   always-on-top/click-through is compositor-dependent).

### What Windows needs beyond the overlay

Everything except the overlay already compiles for `x86_64-pc-windows-msvc`, but two
installer corners are still macOS-shaped. Both need a real Windows machine to settle,
because the answer depends on what is installed there.

- **The status line entry cannot dodge the shell.** Claude Code runs status line commands
  through Git Bash when it is installed and PowerShell when it is not, and unlike hooks the
  `statusLine` schema has no `args` (exec form) escape hatch — so the same string has to
  survive both shells. An unquoted forward-slash path works in either, but a path
  containing a space has no form that satisfies both (Git Bash wants quotes, PowerShell
  needs the call operator). Detecting Git Bash at install time and writing the matching
  form is the likely answer. Hooks already avoid this entirely (see
  `should_write_exec_form_hooks`).
- **The passthrough spawns `/bin/sh` directly** (`statusline.rs`), which does not exist on
  Windows: the user's original status line would silently vanish. Whatever shell is chosen
  has to match the one Claude Code would have used, since the stored command is written in
  that shell's syntax.

Start-at-login is macOS-only too (`setup.rs`), and needs a registry `Run` key or a Startup
shortcut on Windows; the tray's "Start at login" check reports off there until then.

## Layout

```
../Makefile              build / test / install / hooks / statusline / mcp / skill / autostart / render-all
../pets/*.json           the 8 built-in pets (embedded with include_str!)
../skills/hatch-pet/     the /hatch-pet skill for Claude Code
Cargo.toml, integration_test.py
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

Tests: `make test` from the repo root = `cargo test` (300 unit tests, run everywhere; the
overlay's pure modules — row layout, focus/click logic, animation curves, placement,
click/snapshot request parsing — are covered without a window; a few font tests need macOS)
plus `python3 integration_test.py` — a 40-check battery that drives the real binary end to
end (hook lifecycle incl. the approval merge policy, MCP conversation with a real hatched
PNG, installers against fixture files, Swift-format file interop, robustness) inside a
throwaway sandbox. Cross-check without a Mac:
`rustup target add aarch64-apple-darwin && cargo check --target aarch64-apple-darwin`.
