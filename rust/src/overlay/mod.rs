//! The floating pet overlay — macOS implementation (winit + softbuffer + tray-icon).
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
//! 3. Drawing (softbuffer, one RGBA framebuffer): pet sprite via `render::frame_rgba`,
//!    status badge (red clock / orange ? / green check / red !) drawn as simple shapes,
//!    speech bubble with the snapshot message (embedded 8×8 ASCII bitmap font; non-ASCII
//!    replaced with '?'), battery gauge bar under the pet per `AppConfig::gauge_metric`,
//!    session label line. Click on the pet = heart + random `pet_phrases()` line for a
//!    few seconds; drag moves the window.
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

pub fn run() -> i32 {
    todo!("implement the macOS overlay (winit + softbuffer + tray-icon)")
}
