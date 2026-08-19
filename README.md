# claude-airou 🐾

A desktop pet for Claude Code. (Named after the Airou — the Felyne/Palico companion cat from Monster Hunter — because it tags along on your quests.) Like the pets in the OpenAI Codex app, a small pixel-art
creature floats in a corner of your screen and shows what Claude is doing right now:
**thinking / working / waiting for approval / needs your input / done / error**.

- Native macOS overlay written in Rust (winit + a CALayer-presented software compositor + tray menu). One binary, no runtime dependencies.
- Driven by Claude Code **hooks**, so it works with the terminal CLI, the desktop app and IDE extensions alike.
- **Claude chat** (the Claude desktop app) works too: an MCP server (`make mcp`) lets chat drive the pet and hatch new ones — [see below](#using-it-from-claude-chat-the-claude-desktop-app).
- Always on top, never steals focus, visible on every Space and over full-screen apps. Drag it anywhere; it remembers where it was.
- Tracks several sessions at once and puts the one that needs you first.
- Pets are JSON pixel-art packs: 8 built-ins plus your own in `~/.claude-airou/pets/*.json`.
  A `/hatch-pet` skill lets Claude design new ones for you (the counterpart of Codex's `/hatch`).

[한국어 README](README.ko.md)

![states](docs/states.png)

Built-in pets — **Airou** (the namesake and default: a Felyne-style hunting cat, `docs/make_airou.py`) · Mochi (cat) · Quackers (duck) · Boo (ghost) · Jelly (slime) · Bolt (robot) · Inky (octopus) · **Clawd** (the little orange creature from the Claude Code welcome screen, `docs/make_clawd.py`):

![pets](docs/pets.png)

## Install

Requirements: macOS 14+. Nothing else — the installer downloads a prebuilt universal binary, so you do not need Rust or Xcode.

```bash
curl -fsSL https://raw.githubusercontent.com/hyungwookc07/claude-airou/main/install.sh | sh
```

That installs `~/.local/bin/claude-airou`, registers the Claude Code hook in `~/.claude/settings.json` (a backup is written first), installs the `/hatch-pet` skill and puts the pet on screen. Re-run the same line any time to **update**.

**That is the whole of what it changes.** Start-at-login, the status line feed and the MCP server are switches in the menu bar 🐾 menu, so nothing you did not agree to gets registered behind your back:

| Menu bar 🐾 | |
|---|---|
| **Start at login** | off by default — turn it on and the pet comes back after a reboot |
| **Install MCP server for Claude chat…** | lets the Claude desktop app drive the pet |
| **Gauge → Install status line…** | feeds the battery gauge from your Claude Code status line |

Claude Code sessions that were already open do not re-read the hook settings; **start a new session** and the pet will react.

The same switches from a terminal, if you prefer:

```bash
claude-airou setup --with-autostart   # start the overlay at login
claude-airou install-statusline       # feed the battery gauge from your status line (see below)
claude-airou install-mcp              # let Claude chat (the desktop app) drive the pet (see below)
```

Installer options — `curl … | sh -s -- --with-autostart`, `--with-statusline`, `--with-mcp`, `--no-setup` (binary only, changes no settings). `CLAUDE_AIROU_INSTALL_DIR` picks a different directory, `CLAUDE_AIROU_VERSION=vX.Y.Z` a specific release.

Uninstall with `claude-airou uninstall` (removes the hook, status line, MCP entry, skill and login item; your pets and config in `~/.claude-airou` stay), then delete the binary.

<details>
<summary>Build from source instead</summary>

Needs a Rust toolchain ([rustup](https://rustup.rs), stable ≥ 1.85).

```bash
git clone https://github.com/hyungwookc07/claude-airou.git && cd claude-airou
make setup        # build → ~/.local/bin/claude-airou, then hooks + skill
make statusline   # optional
make mcp          # optional
claude-airou        # start the overlay (menu bar 🐾 icon + the pet)
```

Individual targets (`make install`, `make hooks`, `make skill`, `make autostart`, `make no-autostart`, `make uninstall`) still exist; each one is documented in the Makefile.
</details>

"Start at login" registers the LaunchAgent `dev.claude-airou.overlay` (the same thing `make autostart` does; `make no-autostart` removes it).
Only one overlay runs at a time — a second `claude-airou run` prints "already running" and exits (`~/.claude-airou/overlay.lock`).

## How it works

```
Claude Code ──hook event (JSON on stdin)──▶ claude-airou hook ──▶ ~/.claude-airou/state/<session>.json
                                                                          ▲
                                                 claude-airou (overlay) ────┘ polls every 0.3 s and updates the sprite / bubble
```

| Claude Code hook event | Pet state | Shown as |
|---|---|---|
| `SessionStart` (startup/resume/clear) | hello | 👋 wave (back to idle after a few seconds) |
| `UserPromptSubmit`, `PostToolUse`, `PostToolBatch`, `PreCompact`, `SessionStart(compact)` | thinking | thought dots (the sprite's own animation) |
| `PreToolUse`, `SubagentStart` | working | ⚙️ + a one-line tool summary in the bubble ("Reading foo.swift", "Running: git status") |
| `PermissionRequest`, `Notification(permission_prompt)` | waiting_approval | 🔴 pulsing red clock — **needs your approval** |
| `PreToolUse(AskUserQuestion / ExitPlanMode)`, `Notification(agent_needs_input / elicitation_dialog)`, `Elicitation` | needs_input | 🟠 question mark — your turn |
| `Stop`, `Notification(agent_completed)` | done | ✅ green check + a hop (stays until your next prompt; the bubble hides after 30 s) |
| `PostToolUseFailure`, `StopFailure` | error | ❗ shake (stays until the next event) |
| `Notification(idle_prompt)` | idle | (Claude finished ~60 s ago — clears a stuck busy state; a done/error result stays) |
| `SessionEnd` | (session removed) | |

While a session is thinking or working the pet also shows a soft halo behind itself, sized and brightened by the session's **reasoning effort** — from a faint ring at `low` up to a wide glow at `max`, in your system accent colour, and gone the moment the session finishes or starts waiting on you. The level comes from the transcript (the status line supplies it too when installed), so it works everywhere the pet does, including the desktop app. The menu bar 🐾 menu has a **Hide effort aura** switch.

The hook entry itself is written in **exec form** (`command` + `args`, spawned without a shell) when the installed Claude Code is 2.1.139 or newer, and in shell form otherwise — `claude-airou install-hooks --hook-format exec|shell` overrides the choice. Exec form removes quoting from the picture entirely, which matters on Windows: there hooks run through PowerShell when Git Bash is absent, and PowerShell does not execute a single-quoted path without the call operator.

The hook binary never writes to stdout and always exits 0 (Claude Code feeds some events' hook stdout back into the model context). What it saw is logged to `~/.claude-airou/hook.log` (auto-truncated).

Parallel tool calls and subagents fire hooks with the same `session_id`, so the hook merges instead of blindly overwriting: while a session is waiting for approval or an answer, sibling `PostToolUse` events and subagent events are ignored; the wait clears when *that* tool call (`tool_use_id`) finishes, or on `PostToolBatch` / `Stop` / `UserPromptSubmit` (`rust/src/hook_mapper.rs`).

### Known limitations

- **The clock lingers briefly after you approve.** Claude Code has no "user approved" hook. Approval starts the tool; the state clears when the tool finishes (`PostToolUse`), so approving a long-running command keeps the red clock up until it completes.
- **Deny / Esc have no event.** `Stop` does not fire on interrupts and a denial does not fire `PostToolUseFailure`. The next event (`PostToolBatch`, a new prompt, `idle_prompt` after 60 s) cleans up; failing that, states decay to idle by themselves (20 min for waiting/needs-input, 15 min for busy).
- **Which sessions get a pet.** Every session Claude Code fires hooks for, until its `SessionEnd`. Just *opening* a past session (clicking through the list in the desktop app resumes it) fires `SessionStart` — the pet waves hello and then steps out of the row until that session actually does something (a prompt, a tool call). Sessions that die without a `SessionEnd` (killed terminal, crash) are dropped 2 hours after their last event; a live session that stayed silent that long reappears on its next event.
- To relocate the state directory set `CLAUDE_AIROU_HOME=/path` — the overlay and the hook must both see the same value.

## Usage

```
claude-airou                       run the overlay
claude-airou simulate demo         cycle through every state (no hooks needed)
claude-airou simulate waiting_approval --message "Approve? git push"
claude-airou status                the sessions the overlay currently sees
claude-airou pets                  list available pets
claude-airou validate FILE.json    validate a pet file
claude-airou render PET --out DIR  render every frame to PNG (+ sheet.png)
claude-airou preview PET           ASCII preview
claude-airou snapshot --out a.png  save a PNG of the running overlay (no screen-recording permission needed)
claude-airou install-hooks [--print]   / uninstall-hooks
claude-airou mcp                   MCP server for Claude chat (stdio; the desktop app runs this)
claude-airou install-mcp [--print]     / uninstall-mcp
```

**Click** the pet to pet it (hearts + a line of dialogue), **drag** to move, **right-click** (or the menu bar 🐾) for the menu:
choose pet · size (Small/Medium/Large) · sessions (pin one) · gauge metric · show all sessions side by side · hide speech bubbles · click-through · hide pet · reset position · install hooks · open hook log.

Under each pet: a **battery gauge** (context window remaining by default; switch to the 5-hour / 7-day rate-limit remaining or off in menu → Gauge) and the **session label with the status icon** (red clock = waiting for approval, ⚙️ working, ✅ done, …).

### Battery gauge

Claude Code hands its status line a JSON with `context_window.used_percentage`, `rate_limits.five_hour / seven_day` and `cost`. `make statusline` (or `claude-airou install-statusline`) sets `settings.statusLine` to `claude-airou statusline`, which records those figures per session and then **runs your original status line command with the same stdin** — so your terminal status line looks exactly as before. The original is kept in `~/.claude-airou/statusline-passthrough.json`; `claude-airou uninstall-statusline` restores it.

Sessions that never run a status line (e.g. some desktop-app sessions) still get a context gauge: the hook estimates it from the last assistant message's token usage in the transcript. Rate limits are only known through the status line.

### Several sessions at once

Collapsed, the pet shows the session that matters most (waiting for approval > busy > most recent) with a `project +N` badge — a red dot on it means *another* session is waiting on you. **Click the pet to fan the sessions out**: the current one stays in the middle at full size and the others line up left and right at 70 %, each with its own expression, status badge and project name.

![sessions](docs/sessions.png)

- Click a side pet to **pin** that session as the primary one (overrides the automatic rule; "Sessions → Automatic" in the menu undoes it).
- Click the primary pet again to collapse. The row collapses by itself when only one session is left.
- Menu → "Show all sessions side by side" keeps the row expanded permanently.
- The primary pet stays exactly where it was on screen while the row grows and shrinks around it.

Preferences live in `~/.claude-airou/config.json`.

## Using it from Claude chat (the Claude desktop app)

Claude chat has no hook system, so the pet plugs into chat the way chat allows: as an **MCP server**.

```bash
make mcp     # or: claude-airou install-mcp   (a backup of the config is written first)
```

Quit the Claude desktop app completely (Cmd-Q) and reopen it. The overlay now shows a **Claude Chat** session alongside your Claude Code ones:

- 👋 hello when the app launches.
- While chatting, Claude updates the pet with the `pet_status` tool — thinking / working / ✅ done / ❗ error, and 🟠 `needs_input` when it is waiting for your answer, with a one-line speech bubble.
- `hatch_pet` hatches custom pets straight from a chat: Claude designs the pixel art, the server validates and saves it to `~/.claude-airou/pets/`, and the rendered sprite sheet is returned into the conversation so Claude can look at it and iterate — no file access or `/hatch-pet` skill needed. `list_pets` and `preview_pet` round it off.
- Busy states reset to idle after 3 minutes of silence (chat has no Stop event).

Because there are no hooks, the pet only moves when Claude calls the tools. The server's MCP instructions ask Claude to keep the pet honest at every transition, and it generally does; adding one line to your Claude profile preferences ("keep my desktop pet updated with pet_status while you work") makes it near-deterministic.

Limits: this covers the **desktop app** — claude.ai in a browser cannot reach local processes. Claude Code sessions should keep using hooks (richer signal: approvals, per-tool events); the MCP server is for chat.

Undo with `claude-airou uninstall-mcp`. The app's config (`~/Library/Application Support/Claude/claude_desktop_config.json`) is backed up before every change; the server logs to `~/.claude-airou/mcp.log`.

## Making your own pet

In Claude Code:

```
/hatch-pet a sleepy axolotl who thinks every build will pass
```

The skill writes `~/.claude-airou/pets/<id>.json`, checks it with `claude-airou validate` / `render`, and shows you the sprite sheet.
Pick it from the menu bar 🐾 → Pet (use "Reload pets" if the overlay is already running).

To write one by hand, follow the format in [skills/hatch-pet/SKILL.md](skills/hatch-pet/SKILL.md). In short:

```json
{
  "id": "nori-axolotl", "name": "Nori", "species": "axolotl", "fps": 3,
  "palette": { "k": "#3a2a2a", "p": "#f6a7c1", "w": "#ffffff", "e": "#222222" },
  "phrases": { "pet": ["blub."] },
  "frames": {
    "idle":             [ ["..kk..", ".kppk.", "..kk.."], ["..kk..", ".kppk.", "..kk.."] ],
    "thinking":         [ ["..."] ],
    "working":          [ ["..."] ],
    "waiting_approval": [ ["..."] ],
    "needs_input":      [ ["..."] ],
    "done":             [ ["..."] ],
    "error":            [ ["..."] ],
    "hello":            [ ["..."] ]
  }
}
```

- Palette keys are single characters; `.` / space are transparent. Every frame shares one grid size (16×16 to 24×24 recommended).
- Missing states fall back automatically (`working→thinking→idle`, `hello→done→idle`, …).
- The overlay draws the status badge (red clock, green check, …) itself, so the sprite only needs a change of expression.

The built-in pets live in [`pets/`](pets/) and are embedded into the binary at build time.

## Troubleshooting

- The pet does not react → check that `~/.claude/settings.json` has the `hooks` entries (`claude-airou install-hooks --print` shows the expected shape), that you started a new Claude Code session, and that lines appear in `~/.claude-airou/hook.log`.
- A session seems stuck on "waiting for approval" → the session may have died. It decays to idle after 20 minutes; check with `claude-airou status`, or `rm ~/.claude-airou/state/<id>.json`.
- The overlay went off-screen → menu bar 🐾 → Reset position.
- You turned on click-through and can no longer click the pet → turn it off from the menu bar 🐾.
- The pet ignores Claude chat → make sure the desktop app was fully quit and reopened after `make mcp`, that the app's Settings → Developer lists `claude-airou` as running, and that lines appear in `~/.claude-airou/mcp.log`.

## Uninstall

```bash
claude-airou uninstall            # removes the hooks, status line, MCP entry (with backups), the skill and the LaunchAgent
rm -f ~/.local/bin/claude-airou   # the binary itself
rm -rf ~/.claude-airou            # also removes settings, custom pets and state
```

From a source checkout, `make uninstall` does the same and deletes the binary for you.

## The Rust crate

The app is a single Rust crate in [`rust/`](rust/README.md) (macOS overlay today, Windows and
Linux overlays next; the hook, status line, MCP server and CLI already run everywhere). It
started life as a Swift/AppKit app and was ported 1:1 — the Swift original was removed in
v1.0 and is still in git history (commit `3037817`). Roadmap and known gaps are in the
crate's README. 러스트 공부를 겸해 읽는 가이드는 [`rust/LEARNING.ko.md`](rust/LEARNING.ko.md).

## Development

```bash
make build && ./rust/target/release/claude-airou run   # or: cd rust && cargo run -- run
make test               # cargo test + the end-to-end battery (rust/integration_test.py)
make render-all         # renders every built-in pet to render/<id>/sheet.png
```

Layout: `rust/src/{hook,hook_mapper,mcp,mcp_tools,statusline,install,pets,render,state_store,cli,cli_commands}.rs` + `rust/src/overlay/` (macOS). The event → state mapping lives in one place, `hook_mapper.rs`, together with the merge rules for concurrent events; the chat-side tools are in `mcp_tools.rs`; the pet art is in `pets/`.

## License

MIT — see [LICENSE](LICENSE).
