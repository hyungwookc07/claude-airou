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
| Overlay (floating pet, tray menu) | macOS v0.2 stage 1: runs on real hardware; **transparent window** (CALayer-presented RGBA canvas), **system-font text with Hangul** (ab_glyph over SF / SF Rounded / Helvetica / Apple SD Gothic Neo), Swift-matching bubble / gauge pill / label capsule / status badge, tray menu. Single-pet view only (see limitations) |
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
./target/release/claude-airou run             # the Rust overlay itself (v0.2 stage 1)
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

Done in v0.2 stage 1 (verified on a real Mac next to the Swift overlay):

- **True per-pixel transparency.** `overlay/draw.rs` is a premultiplied-RGBA software
  compositor (anti-aliased capsules, circles, lines, sprite blits, text) and
  `overlay/present_macos.rs` hands the buffer to the winit window's `CALayer` as a
  `CGImage` (BGRA premultiplied, `contentsScale` = window scale factor); the NSWindow is
  non-opaque with a clear background and no shadow. Only drawn pixels are visible — no card.
- **Real text.** `overlay/text.rs` rasterises with `ab_glyph` through a per-style font
  chain (`platform_font_files`, the one platform-specific corner): SF (`opsz` set to the
  Text cut, `wght` medium/semibold/bold), SF Rounded for the label, Helvetica, and
  Apple SD Gothic Neo for Hangul/CJK, with per-glyph fallback and a glyph cache. The
  bubble is the Swift one — 11.5 pt medium, 9 pt / 6 pt padding, radius 9, tail, 300 pt
  max, two lines with `…` truncation; the label is 9.5 pt semibold rounded in a capsule.
- Layout follows `RowLayout` for one card (220 pt minimum width, 66 pt bubble slot, gauge
  pill, 22 pt label slot); the window widens symmetrically for wide bubbles so the pet
  never moves, and the persisted origin is the collapsed (no-bubble) origin.
- Light/dark appearance is picked from `NSApp.effectiveAppearance` (re-checked
  periodically).

Still missing (stage 2):

- Session fan-out row, per-session pinning and the `snapshot` / `click` test commands —
  the focused session (approval-needed > busy > most recent) is shown with a `+N` label.
- SwiftUI niceties: bubble fade/scale animation, done-bounce / error-shake, the floating
  heart on click (the phrase bubble is there), gear-spin and pulse on the status badge.
  Badge icons are hand-drawn shapes rather than SF Symbols.
- `windowOriginY` is stored in winit's top-left convention while the Swift app stores
  AppKit's bottom-left origin, so the *vertical* position is not yet exchanged
  correctly between the two overlays (x is).
- Emoji in bubbles: Apple Color Emoji is a bitmap font `ab_glyph` cannot outline, so
  emoji currently advance as blank space.
- Session cleanup on SIGTERM is best-effort (no signal-handling crate); the overlay's
  stale-state decay covers the gap. Killing the overlay with SIGKILL leaves the pid lock
  behind for up to a day (delete `overlay.lock` or wait) — the Swift app uses `flock`.

## Known deviations from Swift

Deliberate, all edge-case-only (an adversarial review pass hunted for more; anything not
listed here is meant to match 1:1 — file a bug if it doesn't):

- **Files are format-compatible, not byte-identical.** Each binary decodes the other's
  files; key order and float formatting differ (Swift sorts keys and prints `1755500000`,
  serde_json keeps declaration order and prints `1755500000.0`).
- **"1 character" = 1 Unicode scalar, not 1 grapheme.** Bubble truncation and pet
  validation count scalars (`char`), Swift counts grapheme clusters — ZWJ emoji and
  NFD-decomposed text diverge (never a crash). Palette keys are single-scalar in practice.
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
   (Korean bubbles). Stage 2: session fan-out + pinning, `snapshot`/`click` commands,
   autostart. Exit criterion: a week of daily use without missing the Swift overlay.
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
  cli_commands.rs        simulate / pets / validate / render / preview / status
  overlay/               macOS overlay — cfg(target_os): window.rs (layout/paint/events),
                         draw.rs (RGBA compositor), text.rs (ab_glyph font chain),
                         present_macos.rs (CALayer presenter + NSWindow tweaks), tray.rs, lock.rs
```

Tests: `cargo test` (261 unit tests, run everywhere; overlay unit tests compile under the
mac target), plus `python3 integration_test.py` — a 40-check battery that drives the real
binary end to end (hook lifecycle incl. the approval merge policy, MCP conversation with a
real hatched PNG, installers against fixture files, Swift-file interop, robustness) inside
a throwaway sandbox. Cross-check without a Mac:
`rustup target add aarch64-apple-darwin && cargo check --target aarch64-apple-darwin`.
