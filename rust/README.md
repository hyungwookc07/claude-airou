# claude-airou (Rust)

The cross-platform rewrite of claude-airou — macOS first, Windows and Linux next.
**End goal: this crate fully replaces the Swift app.** Until it reaches overlay parity,
both implementations live side by side and share every on-disk format, so you can mix
them freely (e.g. Swift overlay + Rust hook/MCP, or the other way around).

한국어로 코드를 따라 읽고 싶다면 → [LEARNING.ko.md](LEARNING.ko.md) (이 코드베이스를 교재로 쓰는 러스트 학습 가이드).

## Status

| Area | State |
|---|---|
| Core (`hook`, `mcp`, `statusline`, installers, CLI: simulate/pets/validate/render/preview/status) | Ported 1:1 from Swift, 218 unit tests + end-to-end smoke run on Linux CI-style checks |
| Overlay (floating pet, tray menu) | macOS implementation written (winit + softbuffer + tray-icon), compiles warning-free for `aarch64-apple-darwin`, **not yet run on real hardware** |
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
./target/release/claude-airou run             # the Rust overlay itself (v0.1)
```

The CLI surface is identical to the Swift binary (`claude-airou help`). To point Claude Code
hooks / the status line / the Claude desktop app at the Rust binary instead of the Swift one,
run its own installers (they edit the same config files, with backups):

```bash
./target/release/claude-airou install-hooks
./target/release/claude-airou install-statusline
./target/release/claude-airou install-mcp
```

## Known v0.1 limitations (overlay)

- **Opaque card instead of a transparent window.** softbuffer's buffer has no alpha channel,
  so the pet sits on a dark rounded card rather than floating free. Swapping the presenter
  (e.g. wgpu or a CALayer-backed surface) is the planned fix; the drawing code is already
  isolated in `overlay/draw.rs` behind that decision.
- **ASCII speech bubbles.** Bubble text uses an embedded 8×8 bitmap font; non-ASCII
  characters (한글 포함) render as `?` for now.
- Single-pet view only: the focused session (approval-needed > busy > most recent) with a
  `+N` label — the Swift overlay's fan-out row, pinning and snapshot/click test commands are
  not ported yet.
- Session cleanup on SIGTERM is best-effort (no signal-handling crate); the overlay's
  stale-state decay covers the gap.

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

1. **v0.1 (this)** — core parity + first overlay. Validate the overlay on real macOS:
   `cargo run --release` next to real Claude Code sessions.
2. **v0.2 — overlay parity.** True transparency, proper text rendering (system font,
   Korean bubbles), session fan-out + pinning, `snapshot`/`click` commands, autostart.
   Exit criterion: a week of daily use without missing the Swift overlay.
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
  overlay/               macOS overlay (winit, softbuffer, tray-icon) — cfg(target_os)
```

Tests: `cargo test` (runs everywhere; overlay unit tests compile under the mac target).
Cross-check without a Mac: `rustup target add aarch64-apple-darwin && cargo check --target aarch64-apple-darwin`.
