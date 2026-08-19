//! The floating pet overlay — macOS implementation (winit + CALayer presenter + tray-icon).
//! Functional reference: the Swift original's `Sources/ClaudeAirou/UI/` (AppDelegate, OverlayPanel, PetView,
//! PetViewModel, RowLayout). What it does:
//!
//! 1. Window: undecorated, transparent, always-on-top, never activates/steals focus
//!    (activation policy Accessory), visible on all Spaces and over full-screen apps
//!    (NSWindow.collectionBehavior canJoinAllSpaces | fullScreenAuxiliary | stationary —
//!    set via objc2-app-kit on the winit NSWindow handle). Draggable (3 pt threshold, a
//!    shorter press is a click); origin persisted in `AppConfig::{window_origin_x,
//!    window_origin_y}` in AppKit bottom-left screen coordinates, shared with the Swift
//!    app (`placement.rs`). Single instance via `paths::overlay_lock_file()`.
//! 2. State (`logic.rs`): poll `StateStore::load_all()` + `load_all_usage()` every 0.3 s.
//!    Focused session = pinned > attention-needed > busy > most recent. One card per
//!    session when fanned out (click the pet / "Show all sessions side by side"), primary
//!    in the middle and larger, side cards pinned by clicking (`row_layout.rs`); the
//!    panel resizes around the primary pet so it never moves on screen.
//! 3. Drawing (software compositor `draw.rs`, premultiplied RGBA with real alpha,
//!    presented through the NSView's CALayer in `present_macos.rs`): pet sprite via
//!    `render::frame_rgba`, speech bubble over the primary pet (system font via `text.rs`
//!    with Hangul fallback, two lines max), battery gauge pill per card, session label
//!    capsule with the compact status badge. Motion (`animation.rs`): fan-out / fold
//!    spring, done hop, error shake, floating heart on click, bubble fade-in, badge
//!    pop-in, pulsing clock / ?, spinning gear.
//! 4. Tray (menu bar) icon 🐾 via tray-icon/muda, also the pet's right-click menu
//!    (`tray.rs`, Swift menu order): Sessions (pin / automatic), Gauge, fan-out toggle,
//!    Pet, Size, toggles, Reset position, hooks / status line installers, hook log, Quit.
//! 5. Test hooks: `claude-airou snapshot` / `claude-airou click` request files under the
//!    airou home are answered every 0.4 s.
//!
//! Everything here is cfg(target_os = "macos"); keep the platform-specific window tweaks
//! in one place (`present_macos.rs`) so the future Windows/Linux backends only replace
//! that corner.

mod animation;
mod draw;
mod lock;
mod logic;
mod placement;
mod present_macos;
mod row_layout;
mod text;
mod tray;
mod window;

use crate::model::AppConfig;
use crate::pets::PetLibrary;

/// Appends one line to `~/.claude-airou/overlay.log` (same shape as hook.log/mcp.log).
pub(crate) fn log(line: &str) {
    crate::logging::append(&crate::paths::root_dir().join("overlay.log"), line);
}

pub fn run() -> i32 {
    let lock_path = crate::paths::overlay_lock_file();
    let pid = std::process::id();
    if lock::acquire(&lock_path, pid, std::time::SystemTime::now()) == lock::LockOutcome::AlreadyRunning
    {
        crate::logging::eprint_line(&format!(
            "claude-airou: the overlay is already running (lock: {}). Nothing to do.",
            lock_path.display()
        ));
        return 0;
    }

    let config = AppConfig::load();
    let library = PetLibrary::load();
    for problem in &library.load_problems {
        crate::logging::eprint_line(&format!("claude-airou: skipping user pet — {problem}"));
    }
    let selected = library
        .resolve_selected(config.selected_pet_id.as_deref())
        .map(|loaded| loaded.definition.clone());
    let Some(pet) = selected else {
        // Swift exits through NSApp.terminate (status 0) after printing the same line.
        crate::logging::eprint_line(
            "claude-airou: no valid pets found (built-ins failed to load). Exiting.",
        );
        lock::release(&lock_path, pid);
        return 0;
    };

    let code = window::run_overlay(config, library, pet);
    lock::release(&lock_path, pid);
    code
}
