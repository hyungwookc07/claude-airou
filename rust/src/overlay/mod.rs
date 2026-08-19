//! The floating pet overlay — macOS implementation (winit + CALayer presenter + tray-icon).
//! Functional reference: `Sources/ClaudeAirou/UI/` (AppDelegate, OverlayPanel, PetView,
//! PetViewModel). v0.1 scope, in priority order:
//!
//! 1. Window: undecorated, transparent, always-on-top, never activates/steals focus
//!    (activation policy Accessory), visible on all Spaces and over full-screen apps
//!    (NSWindow.collectionBehavior canJoinAllSpaces | fullScreenAuxiliary | stationary —
//!    set via objc2-app-kit on the winit NSWindow handle). Draggable; origin persisted in
//!    `AppConfig::{window_origin_x,window_origin_y}` (shared with the Swift app).
//!    Single instance via `paths::overlay_lock_file()` (pid inside; stale lock replaced,
//!    live pid → print "already running" and exit 0, like the Swift overlay).
//! 2. State: poll `StateStore::load_all()` + `load_all_usage()` every 0.3 s (tick via
//!    ControlFlow::WaitUntil). Focused session = attention-needed > busy > most recent
//!    (`effective_state()`); label "<project>" or "<project> +N" with red-dot semantics
//!    left for later. Sprite animation at the pet's fps over `frames_for(state)`.
//! 3. Drawing (software compositor `draw.rs`, premultiplied RGBA with real alpha,
//!    presented through the NSView's CALayer in `present_macos.rs`): pet sprite via
//!    `render::frame_rgba`, speech bubble (rounded translucent capsule, system font via
//!    `text.rs` with Hangul fallback, two lines max), battery gauge pill per
//!    `AppConfig::gauge_metric`, session label capsule with the compact status badge
//!    (red clock / orange ? / green check / red ! / blue gear / yellow wave). Click on the
//!    pet = random `pet_phrases()` line for a few seconds; drag moves the window.
//! 4. Tray (menu bar) icon 🐾 via tray-icon/muda: Pet submenu (built-ins + user pets,
//!    check the selected one, "Reload pets"), Size (Small 3 / Medium 5 / Large 7),
//!    Gauge submenu, toggles (hide bubble / click-through / hide pet), Reset position,
//!    Quit. Selections persist through `AppConfig` (same keys as Swift).
//!
//! Deliberately deferred (documented in rust/README.md): session fan-out row, snapshot/
//! click request files, per-session pinning.
//!
//! Everything here is cfg(target_os = "macos"); keep the platform-specific window tweaks
//! in one place so the future Windows/Linux backends only replace that corner.

mod draw;
mod lock;
mod logic;
mod present_macos;
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
