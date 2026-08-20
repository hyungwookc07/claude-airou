//! The overlay window and event loop (winit 0.30 + CALayer presenter + tray-icon).
//!
//! The window is a transparent, shadowless, always-on-top winit window whose NSView
//! layer shows the software canvas (`present_macos.rs`), so only the drawn pixels are
//! visible: the row of session cards (pet sprite, battery gauge pill, session label
//! capsule) and the speech bubble floating over the primary pet, exactly like the Swift
//! `PetView`. Geometry comes from `row_layout.rs`, behaviour from `logic.rs`, motion
//! curves from `animation.rs`; window placement uses AppKit screen coordinates
//! (`placement.rs`) so `config.json` positions are shared with the Swift overlay.
//!
//! Layout constants below are the Swift `PetView` values in points; every paint call
//! converts to physical pixels through the window's scale factor.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::ModifiersState;
use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS, WindowAttributesExtMacOS};
use winit::window::{Window, WindowId, WindowLevel};

use super::animation;
use super::draw::{self, Canvas, Color};
use super::logic::{ClickAction, LayoutChange, LayoutInputs, OverlayModel};
use super::placement;
use super::present_macos::{
    accent_color_rgb, is_dark_appearance, main_screen_visible_frame, screen_visible_frames, show_alert,
    window_background_rgb, AppKitWindow, LayerPresenter,
};
use super::row_layout::{
    GridSize, RowCard, RowLayout, AURA_RESERVED_MARGIN, CARD_VERTICAL_SPACING, GAUGE_RESERVED_HEIGHT,
    SESSION_BADGE_RESERVED_HEIGHT,
    SPEECH_BUBBLE_BOTTOM_INSET, SPEECH_BUBBLE_MAX_WIDTH, SPEECH_BUBBLE_RESERVED_HEIGHT,
};
use super::text::{FontStyle, TextRasterizer};
use super::{lock, tray};
use crate::cli_commands::{parse_click_request, ClickTarget};
use crate::model::{now_epoch_secs, AppConfig, GaugeMetric, PetState};
use crate::pets::{PetDefinition, PetLibrary, ResolvedPalette};
use crate::state_store::StateStore;

/// Model tick (Swift `tickIntervalSeconds`): frame advance, reaction expiry, menu events.
const TICK: Duration = Duration::from_millis(100);
/// Session state is re-read every third tick (Swift `stateReloadEveryTicks`).
const STATE_RELOAD_EVERY_TICKS: u64 = 3;
/// `snapshot.request` / `click.request` are polled every 0.4 s (Swift snapshot timer).
const REQUEST_POLL_EVERY_TICKS: u64 = 4;
/// Redraw cadence while a short animation (hop, shake, heart, fan-out) is running.
const ANIMATION_FRAME: Duration = Duration::from_millis(16);
/// Debounced config save after window moves (Swift: 0.6 s).
const CONFIG_SAVE_DEBOUNCE: Duration = Duration::from_millis(600);
/// Refresh the pid lock's mtime roughly every 18 s (180 ticks) so the one-day
/// staleness heuristic keeps other instances out while we are alive.
const LOCK_TOUCH_EVERY_TICKS: u64 = 180;
/// Mouse movement beyond this (points) turns a press into a window drag (Swift: 3 pt).
const DRAG_THRESHOLD_POINTS: f64 = 3.0;

// Swift PetView geometry, in points.
const SPEECH_BUBBLE_MIN_WIDTH: f32 = 40.0;
const SPEECH_BUBBLE_EDGE_MARGIN: f32 = 4.0;
const SPEECH_BUBBLE_HORIZONTAL_PADDING: f32 = 9.0;
const SPEECH_BUBBLE_VERTICAL_PADDING: f32 = 6.0;
const SPEECH_BUBBLE_CORNER_RADIUS: f32 = 9.0;
const SPEECH_BUBBLE_TAIL_WIDTH: f32 = 12.0;
const SPEECH_BUBBLE_TAIL_HEIGHT: f32 = 6.0;
const SPEECH_BUBBLE_FONT_SIZE: f32 = 11.5;
const SPEECH_BUBBLE_MAX_LINES: usize = 2;
const SESSION_LABEL_FONT_SIZE: f32 = 9.5;
const SESSION_LABEL_LEADING_PADDING: f32 = 7.0;
const SESSION_LABEL_TRAILING_PADDING: f32 = 7.0;
const SESSION_LABEL_TRAILING_PADDING_WITH_BADGE: f32 = 4.0;
const SESSION_LABEL_VERTICAL_PADDING: f32 = 2.5;
const SESSION_LABEL_ITEM_SPACING: f32 = 4.0;
const SESSION_LABEL_MAX_WIDTH: f32 = 200.0;
/// SwiftUI's line height for the 9.5 pt rounded label font (measured: 17 pt capsule without a badge).
const SESSION_LABEL_TEXT_LINE_HEIGHT: f32 = 12.0;
const ATTENTION_DOT_DIAMETER: f32 = 6.0;
/// The compact badge: a 10 pt bold SF Symbol whose image frame is ~13 pt, padded by 2 pt
/// (17 pt disc, 14 pt visible inside the 1.5 pt white ring — measured against the Swift overlay).
const STATUS_BADGE_ICON_SIZE: f32 = 10.0;
const STATUS_BADGE_ICON_FRAME: f32 = 13.0;
const STATUS_BADGE_PADDING: f32 = 2.0;
const STATUS_BADGE_RING_WIDTH: f32 = 1.5;
/// The gauge's 12 pt slot in the card; the pill itself is text line height + 3 pt of
/// padding (14 pt / 13 pt compact) and overflows the slot symmetrically like SwiftUI does.
const GAUGE_SLOT_HEIGHT: f32 = GAUGE_RESERVED_HEIGHT - CARD_VERTICAL_SPACING;
const GAUGE_PILL_HEIGHT: f32 = 14.0;
const GAUGE_COMPACT_PILL_HEIGHT: f32 = 13.0;
const GAUGE_BODY_WIDTH: f32 = 24.0;
const GAUGE_BODY_HEIGHT: f32 = 10.0;
const GAUGE_COMPACT_BODY_WIDTH: f32 = 18.0;
const GAUGE_COMPACT_BODY_HEIGHT: f32 = 8.0;
const GAUGE_TIP_WIDTH: f32 = 1.5;
const GAUGE_PERCENT_FONT_SIZE: f32 = 9.0;
const GAUGE_COMPACT_PERCENT_FONT_SIZE: f32 = 8.0;
const GAUGE_METRIC_FONT_SIZE: f32 = 7.5;
const GAUGE_HORIZONTAL_PADDING: f32 = 5.0;
const GAUGE_ITEM_SPACING: f32 = 3.0;
/// Side cards are drawn at 92 % opacity (Swift `.opacity(card.isPrimary ? 1 : 0.92)`).
const SIDE_CARD_OPACITY: f32 = 0.92;
/// Room above a card for the hop (-14 pt) and the floating heart (rises 28 pt from 6 pt above the sprite).
const CARD_CANVAS_TOP_MARGIN: f32 = 48.0;
/// The heart symbol size and its resting offset above the sprite (SF `heart.fill` 14 pt, offset -6).
const HEART_SIZE: f32 = 14.0;
const HEART_RESTING_CENTER_ABOVE_SPRITE_TOP: f32 = -2.0;

/// The appearance-dependent palette (system colours in their light/dark values).
#[derive(Clone, Copy)]
struct Theme {
    window_background: Color,
    primary: Color,
    red: Color,
    orange: Color,
    green: Color,
    blue: Color,
    yellow: Color,
    gray: Color,
    pink: Color,
    accent: Color,
}

impl Theme {
    fn for_appearance(is_dark: bool, accent: Option<(u8, u8, u8)>, window_background: Option<(u8, u8, u8)>) -> Theme {
        let accent_color = accent.map(|(red, green, blue)| Color::rgb(red, green, blue));
        let background_color = window_background.map(|(red, green, blue)| Color::rgb(red, green, blue));
        if is_dark {
            Theme {
                window_background: background_color.unwrap_or(Color::rgb(50, 50, 50)),
                primary: Color::WHITE,
                red: Color::rgb(255, 69, 58),
                orange: Color::rgb(255, 159, 10),
                green: Color::rgb(48, 209, 88),
                blue: Color::rgb(10, 132, 255),
                yellow: Color::rgb(255, 214, 10),
                gray: Color::rgb(152, 152, 157),
                pink: Color::rgb(255, 55, 95),
                accent: accent_color.unwrap_or(Color::rgb(10, 132, 255)),
            }
        } else {
            Theme {
                window_background: background_color.unwrap_or(Color::rgb(236, 236, 236)),
                primary: Color::rgb(0, 0, 0),
                red: Color::rgb(255, 59, 48),
                orange: Color::rgb(255, 149, 0),
                green: Color::rgb(52, 199, 89),
                blue: Color::rgb(0, 122, 255),
                yellow: Color::rgb(255, 204, 0),
                gray: Color::rgb(142, 142, 147),
                pink: Color::rgb(255, 45, 85),
                accent: accent_color.unwrap_or(Color::rgb(0, 122, 255)),
            }
        }
    }

    fn secondary(&self) -> Color {
        self.primary.with_opacity(0.62)
    }

    fn tertiary(&self) -> Color {
        self.primary.with_opacity(0.26)
    }
}

pub fn run_overlay(config: AppConfig, library: PetLibrary, pet: PetDefinition) -> i32 {
    let event_loop = {
        let mut builder = EventLoop::builder();
        builder.with_activation_policy(ActivationPolicy::Accessory);
        match builder.build() {
            Ok(event_loop) => event_loop,
            Err(error) => {
                crate::logging::eprint_line(&format!(
                    "claude-airou: could not start the overlay event loop: {error}"
                ));
                return 1;
            }
        }
    };

    let palette = ResolvedPalette::new(&pet);
    let inputs = layout_inputs_for(&pet, &config);
    let mut model = OverlayModel::new(inputs);
    model.is_always_fanned_out = config.is_sessions_always_expanded;
    let mut app = App {
        config,
        library,
        pet,
        palette,
        model,
        store: StateStore::default(),
        window: None,
        presenter: None,
        appkit_window: None,
        bubble_font: TextRasterizer::load_system(FontStyle::BUBBLE),
        label_font: TextRasterizer::load_system(FontStyle::LABEL),
        badge_font: TextRasterizer::load_system(FontStyle::BADGE),
        theme: Theme::for_appearance(true, None, None),
        tray_icon: None,
        menu_model: None,
        context_menu: None,
        next_tick: Instant::now() + TICK,
        next_animation_frame: None,
        tick_count: 0,
        cursor: (0.0, 0.0),
        mouse_down_position: None,
        modifiers: ModifiersState::empty(),
        last_primary_center_x: 0.0,
        badge_state_by_card: HashMap::new(),
        config_dirty_at: None,
        lock_path: crate::paths::overlay_lock_file(),
        pid: std::process::id(),
        exit_code: 0,
    };
    app.last_primary_center_x = app.model.layout.primary_center_x();

    match event_loop.run_app(&mut app) {
        Ok(()) => app.exit_code,
        Err(error) => {
            crate::logging::eprint_line(&format!("claude-airou: overlay event loop failed: {error}"));
            1
        }
    }
}

fn layout_inputs_for(pet: &PetDefinition, config: &AppConfig) -> LayoutInputs {
    let (width, height) = pet.grid_size();
    LayoutInputs {
        grid: GridSize { width: width.max(1) as u32, height: height.max(1) as u32 },
        pixel_scale: config.pixel_scale as f32,
        shows_gauge: config.gauge_metric != GaugeMetric::Off,
    }
}

struct App {
    config: AppConfig,
    library: PetLibrary,
    pet: PetDefinition,
    palette: ResolvedPalette,
    model: OverlayModel,
    store: StateStore,
    window: Option<Rc<Window>>,
    presenter: Option<LayerPresenter>,
    appkit_window: Option<AppKitWindow>,
    bubble_font: TextRasterizer,
    label_font: TextRasterizer,
    badge_font: TextRasterizer,
    theme: Theme,
    tray_icon: Option<tray_icon::TrayIcon>,
    menu_model: Option<tray::MenuModel>,
    /// The right-click menu currently shown (kept alive while it is up).
    context_menu: Option<tray_icon::menu::Menu>,
    next_tick: Instant,
    /// Set while a short animation runs: the next 60 Hz redraw.
    next_animation_frame: Option<Instant>,
    tick_count: u64,
    /// Cursor position in physical pixels within the content.
    cursor: (f64, f64),
    /// Where the left button went down (physical pixels), until it is released or a drag starts.
    mouse_down_position: Option<(f64, f64)>,
    modifiers: ModifiersState,
    /// The primary card's centre in the previous layout — the anchor kept still on screen.
    last_primary_center_x: f32,
    /// Per card id: the badge state last drawn and when it changed (for the badge pop-in).
    badge_state_by_card: HashMap<String, (PetState, f64)>,
    config_dirty_at: Option<Instant>,
    lock_path: std::path::PathBuf,
    pid: u32,
    exit_code: i32,
}

impl App {
    // MARK: - Layout

    fn scale_factor(&self) -> f32 {
        self.window.as_ref().map(|window| window.scale_factor()).unwrap_or(1.0) as f32
    }

    fn layout_inputs(&self) -> LayoutInputs {
        layout_inputs_for(&self.pet, &self.config)
    }

    /// Physical pixels per pet pixel for a card scale (whole number so the sprite stays crisp).
    fn sprite_pixel_scale(&self, card_pixel_scale: f32) -> u32 {
        ((card_pixel_scale * self.scale_factor()).round() as i64).max(1) as u32
    }

    fn is_bubble_visible(&self) -> bool {
        self.model.is_speech_bubble_visible(self.config.is_speech_bubble_hidden)
    }

    /// Swift `measuredSpeechBubbleWidth(for:)`: single line up to the max, then it wraps.
    fn measured_bubble_width(&mut self, text: &str) -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        let scale = self.scale_factor();
        self.bubble_font.set_size(SPEECH_BUBBLE_FONT_SIZE, scale);
        let text_width = self.bubble_font.measure(text) / scale;
        SPEECH_BUBBLE_MAX_WIDTH.min((text_width + SPEECH_BUBBLE_HORIZONTAL_PADDING * 2.0 + 2.0).ceil())
    }

    fn current_bubble_width(&mut self) -> f32 {
        if self.is_bubble_visible() {
            let text = self.model.speech_text().to_string();
            self.measured_bubble_width(&text)
        } else {
            0.0
        }
    }

    /// Recomputes the row layout and resizes the panel around the primary pet when needed
    /// (Swift `relayoutIfNeeded` + the `layoutDidChange` sink).
    fn relayout(&mut self) {
        let now = now_epoch_secs();
        let bubble_width = self.current_bubble_width();
        let visible = self.is_bubble_visible();
        self.model.note_bubble_visibility(visible, now);
        let inputs = self.layout_inputs();
        match self.model.relayout(inputs, bubble_width, now) {
            LayoutChange::Unchanged => {}
            LayoutChange::ContentOnly => self.request_redraw(),
            LayoutChange::PanelGeometry => self.apply_panel_geometry(),
        }
    }

    /// Resizes the panel so the primary pet stays where it is on screen while the row grows
    /// or shrinks around it, then draws the new content into the new frame right away.
    fn apply_panel_geometry(&mut self) {
        let Some(appkit_window) = &self.appkit_window else { return };
        let previous = appkit_window.frame();
        let anchor_screen_x = previous.min_x() + self.last_primary_center_x as f64;
        let layout = &self.model.layout;
        let new_frame = placement::frame_keeping_content_x(
            previous,
            layout.content_width as f64,
            layout.content_height as f64,
            layout.primary_center_x() as f64,
            anchor_screen_x,
        );
        appkit_window.set_frame(new_frame);
        super::log(&format!(
            "panel resize: {:?} -> {:?} (anchor {anchor_screen_x}, primary {} -> {})",
            previous,
            new_frame,
            self.last_primary_center_x,
            self.model.layout.primary_center_x()
        ));
        self.nudge_onto_screen();
        let Some(appkit_window) = &self.appkit_window else { return };
        self.model.panel_shift_x = (appkit_window.frame().min_x() - previous.min_x()) as f32;
        self.last_primary_center_x = self.model.layout.primary_center_x();
        self.redraw();
        self.schedule_animation_frames();
    }

    fn nudge_onto_screen(&mut self) {
        let Some(appkit_window) = &self.appkit_window else { return };
        let frame = appkit_window.frame();
        let screens = screen_visible_frames();
        let (x, y) = placement::nudged_origin(frame, &screens);
        if (x, y) != (frame.x, frame.y) {
            super::log(&format!("nudge: {frame:?} -> ({x}, {y}) screens {screens:?}"));
            appkit_window.set_frame_origin(x, y);
        }
    }

    // MARK: - Startup

    fn create_window(&mut self, event_loop: &ActiveEventLoop) {
        let initial_layout = self.model.layout.clone();
        let attributes = Window::default_attributes()
            .with_title("Claude Airou")
            .with_inner_size(LogicalSize::new(initial_layout.content_width, initial_layout.content_height))
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_visible(false)
            .with_has_shadow(false)
            .with_accepts_first_mouse(true);
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Rc::new(window),
            Err(error) => {
                crate::logging::eprint_line(&format!(
                    "claude-airou: could not create the overlay window: {error}"
                ));
                self.exit_code = 1;
                event_loop.exit();
                return;
            }
        };
        match (LayerPresenter::attach(&window), AppKitWindow::from_winit(&window)) {
            (Some(presenter), Some(appkit_window)) => {
                self.presenter = Some(presenter);
                self.appkit_window = Some(appkit_window);
            }
            _ => {
                crate::logging::eprint_line("claude-airou: could not attach the overlay's content layer");
                self.exit_code = 1;
                event_loop.exit();
                return;
            }
        }
        if self.config.is_click_through {
            let _ = window.set_cursor_hittest(false);
        }
        self.window = Some(window.clone());
        self.refresh_theme();

        // Swift `panel.place(at: savedOrigin)`: the saved origin if it keeps the panel on
        // some screen, otherwise the default corner. AppKit coordinates throughout.
        let saved_origin = match (self.config.window_origin_x, self.config.window_origin_y) {
            (Some(x), Some(y)) => Some((x, y)),
            _ => None,
        };
        if let Some(appkit_window) = &self.appkit_window {
            let frame = appkit_window.frame();
            let (x, y) = placement::placement_origin(
                saved_origin,
                frame.width,
                frame.height,
                &screen_visible_frames(),
                main_screen_visible_frame(),
            );
            appkit_window.set_frame_origin(x, y);
        }
        self.redraw();
        if !self.config.is_pet_hidden {
            window.set_visible(true);
        }
        super::log(&format!("overlay started (pid {}, pet {})", self.pid, self.pet.id));
    }

    /// Re-reads the appearance and the system colours the theme depends on.
    fn refresh_theme(&mut self) {
        let is_dark = is_dark_appearance();
        let accent = accent_color_rgb();
        let background = window_background_rgb();
        self.theme = Theme::for_appearance(is_dark, accent, background);
        if self.tick_count == 0 {
            super::log(&format!("theme: dark={is_dark} accent={accent:?} windowBackground={background:?}"));
        }
    }

    fn create_tray(&mut self) {
        let size = 22u32;
        let rgba = tray::paw_icon_rgba(size);
        let Ok(icon) = tray_icon::Icon::from_rgba(rgba, size, size) else {
            super::log("tray: could not build the paw icon");
            return;
        };
        let menu = tray::build_menu(&self.current_menu_model());
        match tray_icon::TrayIconBuilder::new()
            .with_icon(icon)
            .with_icon_as_template(true)
            .with_tooltip("Claude Airou")
            .with_menu(Box::new(menu))
            .build()
        {
            Ok(tray_icon) => {
                self.menu_model = Some(self.current_menu_model());
                self.tray_icon = Some(tray_icon);
            }
            Err(error) => super::log(&format!("tray: could not create the status item: {error}")),
        }
    }

    // MARK: - Menu (tray + right-click)

    fn current_menu_model(&self) -> tray::MenuModel {
        let selected_id = self.pet.id.as_str();
        tray::MenuModel {
            header: self.model.menu_header(&self.pet.name),
            usage_line: self
                .model
                .usage_summary(self.model.focused.as_ref().map(|session| session.session_id.as_str())),
            sessions: self
                .model
                .sessions
                .iter()
                .map(|session| tray::SessionMenuRow {
                    session_id: session.session_id.clone(),
                    title: format!("{} — {}", session.project_name(), session.effective_state().display_label()),
                    is_pinned: Some(&session.session_id) == self.model.pinned_session_id.as_ref(),
                })
                .collect(),
            is_following_automatically: self.model.pinned_session_id.is_none(),
            pets: tray::MenuModel::pet_rows(&self.library, selected_id),
            pixel_scale: self.config.pixel_scale,
            gauge_metric: self.config.gauge_metric,
            always_expanded: self.config.is_sessions_always_expanded,
            bubbles_hidden: self.config.is_speech_bubble_hidden,
            click_through: self.config.is_click_through,
            pet_hidden: self.config.is_pet_hidden,
            start_at_login: crate::setup::is_login_autostart_installed(),
            effort_aura_hidden: self.config.is_effort_aura_hidden,
        }
    }

    fn sync_tray_menu(&mut self) {
        let Some(tray_icon) = &self.tray_icon else { return };
        let fresh = self.current_menu_model();
        if self.menu_model.as_ref() == Some(&fresh) {
            return;
        }
        tray_icon.set_menu(Some(Box::new(tray::build_menu(&fresh))));
        self.menu_model = Some(fresh);
    }

    /// Right-click / control-click on the pet: the same menu as the tray icon, at the mouse.
    fn show_context_menu(&mut self) {
        let Some(appkit_window) = &self.appkit_window else { return };
        let menu = tray::build_menu(&self.current_menu_model());
        appkit_window.show_context_menu(&menu);
        self.context_menu = Some(menu);
    }

    fn handle_menu_action(&mut self, action: tray::MenuAction, event_loop: &ActiveEventLoop) {
        use tray::MenuAction::*;
        super::log(&format!("menu: {}", tray::menu_id_for(&action)));
        match action {
            PinSession(session_id) => {
                self.model.pin(Some(session_id));
                self.relayout();
            }
            FollowSessionsAutomatically => {
                self.model.pin(None);
                self.relayout();
            }
            SelectPet(pet_id) => {
                if let Some(loaded) = self.library.pet_with_id(&pet_id) {
                    self.pet = loaded.definition.clone();
                    self.palette = ResolvedPalette::new(&self.pet);
                    self.model.frame_index = 0;
                    self.config.selected_pet_id = Some(pet_id);
                    self.config.save();
                    self.relayout();
                    self.request_redraw();
                }
            }
            SelectSize(scale) => {
                self.config.pixel_scale = scale;
                self.config.save();
                self.relayout();
            }
            SelectGauge(metric) => {
                self.config.gauge_metric = metric;
                self.config.save();
                self.relayout();
                self.request_redraw();
            }
            InstallStatusLine => match crate::install::install_statusline_at_default_paths() {
                Ok(summary) => show_alert(
                    "Status line wired to claude-airou",
                    &format!("{summary}\n\nNew Claude Code sessions feed the gauge; your own status line keeps rendering."),
                    false,
                ),
                Err(error) => show_alert("Could not install the status line feed", &error, true),
            },
            ToggleAlwaysExpanded => {
                self.config.is_sessions_always_expanded = !self.config.is_sessions_always_expanded;
                self.model.is_always_fanned_out = self.config.is_sessions_always_expanded;
                self.config.save();
                self.relayout();
            }
            ToggleEffortAura => {
                self.config.is_effort_aura_hidden = !self.config.is_effort_aura_hidden;
                self.config.save();
                self.request_redraw();
            }
            ToggleBubbles => {
                self.config.is_speech_bubble_hidden = !self.config.is_speech_bubble_hidden;
                self.config.save();
                self.relayout();
                self.request_redraw();
            }
            ToggleClickThrough => {
                self.config.is_click_through = !self.config.is_click_through;
                if let Some(window) = &self.window {
                    let _ = window.set_cursor_hittest(!self.config.is_click_through);
                }
                self.config.save();
            }
            TogglePetHidden => {
                self.config.is_pet_hidden = !self.config.is_pet_hidden;
                if let Some(window) = &self.window {
                    window.set_visible(!self.config.is_pet_hidden);
                }
                self.config.save();
            }
            ReloadPets => self.reload_pets(),
            OpenPetsFolder => {
                let directory = crate::paths::pets_dir();
                let _ = crate::paths::ensure_dir(&directory);
                open_with_finder(&directory);
            }
            ResetPosition => self.reset_position(),
            InstallHooks => match crate::install::install_hooks_at_default_paths() {
                Ok(summary) => show_alert("Claude Code hooks installed", &summary, false),
                Err(error) => show_alert("Could not install hooks", &error, true),
            },
            InstallMcp => match crate::install::install_mcp_at_default_paths() {
                Ok(summary) => show_alert(
                    "MCP server registered",
                    &format!("{summary}\n\nQuit the Claude desktop app completely (Cmd-Q) and reopen it for the server to load."),
                    false,
                ),
                Err(error) => show_alert("Could not register the MCP server", &error, true),
            },
            ToggleStartAtLogin => {
                let is_enabled = crate::setup::is_login_autostart_installed();
                let outcome = if is_enabled {
                    crate::setup::remove_login_autostart_at_default_paths()
                } else {
                    crate::setup::install_login_autostart_at_default_paths()
                };
                if let Err(error) = outcome {
                    let title = if is_enabled {
                        "Could not turn off start at login"
                    } else {
                        "Could not turn on start at login"
                    };
                    show_alert(title, &error, true);
                }
            }
            OpenHookLog => {
                let log_path = crate::paths::hook_log_file();
                if !log_path.exists() {
                    if let Some(parent) = log_path.parent() {
                        let _ = crate::paths::ensure_dir(parent);
                    }
                    let _ = std::fs::write(&log_path, b"");
                }
                open_with_finder(&log_path);
            }
            Quit => self.quit(event_loop),
        }
        self.sync_tray_menu();
    }

    fn reload_pets(&mut self) {
        self.library = PetLibrary::load();
        for problem in &self.library.load_problems {
            crate::logging::eprint_line(&format!("claude-airou: skipping user pet — {problem}"));
        }
        let current_id = self.pet.id.clone();
        let replacement = self
            .library
            .pet_with_id(&current_id)
            .or_else(|| self.library.pets.first())
            .map(|loaded| loaded.definition.clone());
        if let Some(definition) = replacement {
            self.pet = definition;
            self.palette = ResolvedPalette::new(&self.pet);
            self.model.frame_index = 0;
            self.relayout();
            self.request_redraw();
        }
    }

    /// Bottom-right corner of the main screen's visible area with a 24-point margin
    /// (Swift `OverlayPanel.defaultOrigin(for:)`).
    fn reset_position(&mut self) {
        let Some(appkit_window) = &self.appkit_window else { return };
        let frame = appkit_window.frame();
        let (x, y) = placement::default_origin(frame.width, main_screen_visible_frame());
        appkit_window.set_frame_origin(x, y);
        self.remember_origin();
    }

    /// Persists where the *collapsed* panel would sit (Swift `onDidMove`): the primary pet's
    /// screen x minus the collapsed layout's primary centre, and the AppKit bottom-left y.
    /// The collapsed layout is measured without a bubble, so a wide bubble at quit time
    /// cannot shift the pet on the next launch.
    fn remember_origin(&mut self) {
        let Some(appkit_window) = &self.appkit_window else { return };
        let frame = appkit_window.frame();
        let primary_screen_x = frame.min_x() + self.model.layout.primary_center_x() as f64;
        let collapsed_center_x = self.model.collapsed_layout(self.layout_inputs(), 0.0).primary_center_x() as f64;
        let origin_x = primary_screen_x - collapsed_center_x;
        let origin_y = frame.min_y();
        if self.config.window_origin_x != Some(origin_x) || self.config.window_origin_y != Some(origin_y) {
            self.config.window_origin_x = Some(origin_x);
            self.config.window_origin_y = Some(origin_y);
            self.config_dirty_at = Some(Instant::now());
        }
    }

    fn quit(&mut self, event_loop: &ActiveEventLoop) {
        if self.config_dirty_at.take().is_some() {
            self.config.save();
        }
        lock::release(&self.lock_path, self.pid);
        super::log("overlay quit");
        event_loop.exit();
    }

    // MARK: - Tick

    fn tick(&mut self, event_loop: &ActiveEventLoop) {
        self.tick_count = self.tick_count.wrapping_add(1);
        let now = now_epoch_secs();
        let mut needs_redraw = false;

        if self.tick_count.is_multiple_of(STATE_RELOAD_EVERY_TICKS) {
            let sessions = self.store.load_all();
            let usage = self.store.load_all_usage();
            if self.model.reload(sessions, usage, now) {
                needs_redraw = true;
            }
        }
        if self.model.advance_frames(TICK.as_secs_f64(), self.pet.frames_per_second()) {
            needs_redraw = true;
        }
        if self.model.expire_pet_reaction_if_due(now) {
            needs_redraw = true;
        }
        if self.model.finish_collapse_if_due(now) {
            needs_redraw = true;
        }

        // Menu clicks arrive on muda's global channel; drain it here.
        while let Ok(event) = tray_icon::menu::MenuEvent::receiver().try_recv() {
            if let Some(action) = tray::action_for_menu_id(event.id.0.as_str()) {
                self.handle_menu_action(action, event_loop);
                if event_loop.exiting() {
                    return;
                }
            }
        }
        // Drain (and ignore) raw tray icon events so the channel never backs up.
        while tray_icon::TrayIconEvent::receiver().try_recv().is_ok() {}

        if self.tick_count.is_multiple_of(REQUEST_POLL_EVERY_TICKS) {
            self.answer_requests_if_any();
        }
        if self.tick_count.is_multiple_of(LOCK_TOUCH_EVERY_TICKS) {
            lock::touch(&self.lock_path, self.pid);
            self.refresh_theme();
        }
        if let Some(dirty_at) = self.config_dirty_at {
            if dirty_at.elapsed() >= CONFIG_SAVE_DEBOUNCE {
                self.config_dirty_at = None;
                self.config.save();
            }
        }

        self.sync_tray_menu();
        // The bubble text and the session list drive the geometry, so re-layout every tick.
        self.relayout();
        // Continuous effects (badge pulse, gear spin) redraw at the tick rate.
        if needs_redraw || self.has_continuous_effect() {
            self.request_redraw();
        }
        self.schedule_animation_frames();
    }

    /// `symbolEffect(.pulse)` / the spinning gear never stop while the state lasts.
    fn has_continuous_effect(&self) -> bool {
        self.model.layout.cards.iter().any(|card| {
            matches!(
                self.model.state_for_card(card),
                PetState::WaitingApproval | PetState::NeedsInput | PetState::Working
            )
        })
    }

    fn schedule_animation_frames(&mut self) {
        if self.model.is_animating(now_epoch_secs()) {
            if self.next_animation_frame.is_none() {
                self.next_animation_frame = Some(Instant::now() + ANIMATION_FRAME);
            }
        } else {
            self.next_animation_frame = None;
        }
    }

    /// Answers `click.request` and `snapshot.request` (Swift `answerSnapshotRequestIfAny`).
    fn answer_requests_if_any(&mut self) {
        let click_path = crate::paths::click_request_file();
        if let Ok(data) = std::fs::read(&click_path) {
            let _ = std::fs::remove_file(&click_path);
            let text = String::from_utf8_lossy(&data).to_string();
            match parse_click_request(&text) {
                Some(ClickTarget::ContentX(x)) => self.handle_click(x as f32),
                Some(ClickTarget::Primary) => self.handle_click(self.model.layout.primary_center_x()),
                None => super::log(&format!("click request ignored: {text:?}")),
            }
        }

        let request_path = crate::paths::snapshot_request_file();
        if !request_path.exists() {
            return;
        }
        let _ = std::fs::remove_file(&request_path);
        match self.render_snapshot_png() {
            Ok(png_bytes) => {
                if let Err(error) = crate::state_store::write_atomic(&crate::paths::snapshot_image_file(), &png_bytes) {
                    crate::logging::eprint_line(&format!("claude-airou: snapshot failed: {error}"));
                }
            }
            Err(error) => crate::logging::eprint_line(&format!("claude-airou: snapshot failed: {error}")),
        }
    }

    /// Renders the panel's own content to PNG bytes (no screen-recording permission needed).
    fn render_snapshot_png(&mut self) -> Result<Vec<u8>, String> {
        let Some(window) = self.window.clone() else { return Err("no window".to_string()) };
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Err("empty window".to_string());
        }
        let mut canvas = Canvas::new(size.width, size.height);
        self.paint(&mut canvas, now_epoch_secs());
        canvas.encode_png()
    }

    // MARK: - Drawing

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn redraw(&mut self) {
        let Some(window) = self.window.clone() else { return };
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        let mut canvas = Canvas::new(size.width, size.height);
        self.paint(&mut canvas, now_epoch_secs());
        if let Some(presenter) = &self.presenter {
            presenter.present(&canvas, window.scale_factor());
        }
    }

    /// Paints the whole overlay: every card (bottom-aligned in the row under the bubble
    /// slot), then the speech bubble over the primary card.
    fn paint(&mut self, canvas: &mut Canvas, now: f64) {
        let scale = self.scale_factor();
        let layout = self.model.layout.clone();
        let card_height = layout.card_height();
        let cards_bottom = SPEECH_BUBBLE_RESERVED_HEIGHT + card_height;

        for card in &layout.cards {
            let card_canvas = self.paint_card(card, &layout, now);
            let (offset_x, motion_scale, motion_opacity) = self.card_transform(card, &layout, now);
            let opacity = motion_opacity * if card.is_primary { 1.0 } else { SIDE_CARD_OPACITY };
            let canvas_width_points = card_canvas.width as f32 / scale;
            let canvas_height_points = card_canvas.height as f32 / scale;
            let center_x = card.center_x() + offset_x;
            let destination_left = center_x - canvas_width_points * motion_scale / 2.0;
            let destination_top = cards_bottom - canvas_height_points * motion_scale;
            canvas.blit_canvas(
                &card_canvas,
                (destination_left * scale).round(),
                (destination_top * scale).round(),
                motion_scale,
                opacity,
            );
        }

        if self.is_bubble_visible() {
            self.paint_speech_bubble(canvas, &layout, now);
        }
    }

    /// (offset x in points, scale about the bottom centre, opacity) for a card's motion:
    /// folding into the primary while collapsing, or settling from its previous slot.
    fn card_transform(&self, card: &RowCard, layout: &RowLayout, now: f64) -> (f32, f32, f32) {
        if self.model.is_collapsing && !card.is_primary {
            if let Some(elapsed) = self.model.collapse_elapsed_secs(now) {
                return animation::card_fold_at(card.center_x(), layout.primary_center_x(), elapsed as f32);
            }
        }
        if let Some(previous) = &self.model.previous_layout {
            let elapsed = (now - self.model.layout_changed_at_secs) as f32;
            if !animation::is_spring_settled(elapsed, animation::CARD_SETTLE_DURATION_SECS) {
                if let Some(start) = animation::card_motion_start(previous, layout, card.id(), self.model.panel_shift_x) {
                    return animation::card_motion_at(start, elapsed);
                }
            }
        }
        (0.0, 1.0, 1.0)
    }

    /// Paints one session card (sprite, gauge, label) into its own canvas so it can be
    /// composited with a scale/opacity. The canvas is wider than the card (labels may
    /// overflow up to 200 pt) and taller (room for the hop and the heart above the sprite).
    fn paint_card(&mut self, card: &RowCard, layout: &RowLayout, now: f64) -> Canvas {
        let scale = self.scale_factor();
        // The aura margin is part of the buffer on every side: the canvas is centred on the
        // card, so a wider buffer just means the halo has somewhere to go.
        let canvas_width_points =
            card.width.max(SESSION_LABEL_MAX_WIDTH + 8.0) + AURA_RESERVED_MARGIN * 2.0;
        let card_height = layout.card_height();
        let canvas_height_points = CARD_CANVAS_TOP_MARGIN + AURA_RESERVED_MARGIN + card_height;
        let mut card_canvas = Canvas::new(
            (canvas_width_points * scale).round() as u32,
            (canvas_height_points * scale).round() as u32,
        );
        let center_x = canvas_width_points / 2.0;
        let bottom = canvas_height_points;
        let state = self.model.state_for_card(card);

        // From the bottom: label slot (22), spacing, gauge pill (12), spacing, sprite.
        let label_center_y = bottom - SESSION_BADGE_RESERVED_HEIGHT / 2.0;
        let mut sprite_bottom = bottom - SESSION_BADGE_RESERVED_HEIGHT - CARD_VERTICAL_SPACING;
        if layout.shows_gauge {
            let gauge_slot_top = sprite_bottom - GAUGE_SLOT_HEIGHT;
            let value = self.model.gauge_value_for_card(card, self.config.gauge_metric);
            self.paint_gauge(&mut card_canvas, center_x, gauge_slot_top + GAUGE_SLOT_HEIGHT / 2.0, value, !card.is_primary);
            sprite_bottom = gauge_slot_top - CARD_VERTICAL_SPACING;
        }

        // Sprite (primary: hop / shake offsets and the floating heart).
        let sprite_pixel_scale = self.sprite_pixel_scale(card.pixel_scale);
        let grid = self.layout_inputs().grid;
        let sprite_width_points = grid.width as f32 * card.pixel_scale;
        let sprite_height_points = grid.height as f32 * card.pixel_scale;
        let mut sprite_offset_x = 0.0;
        let mut sprite_offset_y = 0.0;
        if card.is_primary {
            if let Some(started_at) = self.model.done_bounce_started_at_secs {
                if let Some(offset_y) = animation::done_bounce_offset_y((now - started_at) as f32) {
                    sprite_offset_y = offset_y;
                }
            }
            if let Some(started_at) = self.model.error_shake_started_at_secs {
                if let Some(offset_x) = animation::error_shake_offset_x((now - started_at) as f32) {
                    sprite_offset_x = offset_x;
                }
            }
        }
        let sprite_left = ((center_x - sprite_width_points / 2.0 + sprite_offset_x) * scale).round() as i32;
        let sprite_top = ((sprite_bottom - sprite_height_points + sprite_offset_y) * scale).round() as i32;
        // Aura first, so the sprite sits on top of its own glow.
        if let Some(effort) = self.model.effort_aura_for_card(card, self.config.is_effort_aura_hidden) {
            let (inner_scale, outer_scale, opacity) = effort.aura_radii_and_opacity();
            let sprite_center_x = center_x + sprite_offset_x;
            let sprite_center_y = sprite_bottom - sprite_height_points / 2.0 + sprite_offset_y;
            let half_height = sprite_height_points / 2.0;
            card_canvas.fill_radial_glow(
                sprite_center_x * scale,
                sprite_center_y * scale,
                half_height * inner_scale * scale,
                half_height * outer_scale * scale,
                self.theme.accent.with_opacity(if card.is_primary { opacity } else { opacity * 0.6 }),
            );
        }

        let frames = self.pet.frames_for(state);
        if !frames.is_empty() {
            let frame = &frames[self.model.frame_index % frames.len()];
            if let Ok((rgba, image_width, image_height)) =
                crate::render::frame_rgba(frame, &self.palette, sprite_pixel_scale, None)
            {
                card_canvas.blit_rgba(&rgba, image_width, image_height, sprite_left, sprite_top);
            }
        }
        if card.is_primary {
            if let Some(started_at) = self.model.pet_reaction_started_at_secs {
                if let Some((rise, opacity)) = animation::floating_heart((now - started_at) as f32) {
                    let heart_center_y = sprite_bottom - sprite_height_points + HEART_RESTING_CENTER_ABOVE_SPRITE_TOP - rise;
                    card_canvas.fill_heart(
                        center_x * scale,
                        heart_center_y * scale,
                        HEART_SIZE * scale,
                        self.theme.pink.with_opacity(opacity),
                    );
                }
            }
        }

        // Session label capsule with the compact status badge.
        let is_expanded = self.model.is_expanded();
        let (text, is_dimmed, is_highlighted, has_attention_dot) = if card.is_primary {
            (
                if is_expanded { card.label.clone() } else { self.model.collapsed_label() },
                self.model.focused.is_none(),
                is_expanded,
                !is_expanded && self.model.has_hidden_attention(),
            )
        } else {
            (card.label.clone(), false, false, false)
        };
        self.paint_session_label(
            &mut card_canvas,
            card.id(),
            &text,
            state,
            is_dimmed,
            is_highlighted,
            has_attention_dot,
            center_x,
            label_center_y,
            now,
        );
        card_canvas
    }

    /// Swift `SpeechBubble`, centred over the primary pet (RowLayout reserves the room),
    /// clamped inside the content as a safety net; plays the fade/scale entrance.
    fn paint_speech_bubble(&mut self, canvas: &mut Canvas, layout: &RowLayout, now: f64) {
        let scale = self.scale_factor();
        let content_width = canvas.width as f32 / scale;
        self.bubble_font.set_size(SPEECH_BUBBLE_FONT_SIZE, scale);
        let text = self.model.speech_text().to_string();
        let bubble_width = layout
            .speech_bubble_width
            .max(SPEECH_BUBBLE_MIN_WIDTH)
            .min(content_width - 2.0 * SPEECH_BUBBLE_EDGE_MARGIN);
        let text_width_limit = (bubble_width - 2.0 * SPEECH_BUBBLE_HORIZONTAL_PADDING) * scale;
        let font = &mut self.bubble_font;
        let lines = draw::wrap_text(&text, text_width_limit, SPEECH_BUBBLE_MAX_LINES, |candidate| font.measure(candidate));
        let line_height = self.bubble_font.line_height() / scale;
        let bubble_height = lines.len() as f32 * line_height + 2.0 * SPEECH_BUBBLE_VERTICAL_PADDING;
        let half_width = bubble_width / 2.0;
        let bubble_center_x = layout
            .primary_center_x()
            .max(half_width + SPEECH_BUBBLE_EDGE_MARGIN)
            .min(content_width - half_width - SPEECH_BUBBLE_EDGE_MARGIN);
        let bubble_bottom = SPEECH_BUBBLE_RESERVED_HEIGHT - SPEECH_BUBBLE_BOTTOM_INSET;

        // Paint into a bubble-sized canvas (with margins for shadow and tail) so the
        // entrance can scale it about its bottom centre and fade it in.
        let margin = 3.0;
        let bubble_canvas_width = bubble_width + 2.0 * margin;
        let bubble_canvas_height = bubble_height + margin + SPEECH_BUBBLE_TAIL_HEIGHT;
        let mut bubble_canvas = Canvas::new(
            (bubble_canvas_width * scale).ceil() as u32,
            (bubble_canvas_height * scale).ceil() as u32,
        );
        let left = margin;
        let top = margin;
        let fill = self.theme.window_background.with_opacity(0.94);
        // Soft shadow (Swift: black 18 %, radius 3, y offset 1), approximated by one halo.
        bubble_canvas.fill_rounded_rect(
            (left - 1.0) * scale,
            (top + 0.5) * scale,
            (bubble_width + 2.0) * scale,
            (bubble_height + 2.0) * scale,
            (SPEECH_BUBBLE_CORNER_RADIUS + 1.0) * scale,
            Color::rgba(0, 0, 0, 24),
        );
        bubble_canvas.fill_rounded_rect(
            left * scale,
            top * scale,
            bubble_width * scale,
            bubble_height * scale,
            SPEECH_BUBBLE_CORNER_RADIUS * scale,
            fill,
        );
        bubble_canvas.stroke_rounded_rect(
            left * scale,
            top * scale,
            bubble_width * scale,
            bubble_height * scale,
            SPEECH_BUBBLE_CORNER_RADIUS * scale,
            1.0 * scale,
            self.theme.primary.with_opacity(0.12),
        );
        // Tail: 12×6 triangle hanging 5.5 pt below the bubble (overlaps 0.5 pt so it joins).
        bubble_canvas.fill_triangle_down(
            (left + half_width - SPEECH_BUBBLE_TAIL_WIDTH / 2.0) * scale,
            (top + bubble_height - 0.5) * scale,
            SPEECH_BUBBLE_TAIL_WIDTH * scale,
            SPEECH_BUBBLE_TAIL_HEIGHT * scale,
            fill,
        );
        let text_left = (left + SPEECH_BUBBLE_HORIZONTAL_PADDING) * scale;
        let mut text_top = (top + SPEECH_BUBBLE_VERTICAL_PADDING) * scale;
        for line in &lines {
            bubble_canvas.draw_text(&mut self.bubble_font, line, text_left, text_top, self.theme.primary);
            text_top += line_height * scale;
        }

        // Entrance: scale about the tail tip / bottom edge, fade in.
        let elapsed = self.model.bubble_shown_at_secs.map(|shown_at| (now - shown_at) as f32).unwrap_or(1.0);
        let (entrance_scale, entrance_opacity) = animation::bubble_appearance(elapsed);
        let anchor_x = bubble_center_x;
        let anchor_y = bubble_bottom + SPEECH_BUBBLE_TAIL_HEIGHT - 0.5;
        let destination_left = anchor_x - bubble_canvas_width * entrance_scale / 2.0;
        let destination_top = anchor_y - bubble_canvas_height * entrance_scale;
        canvas.blit_canvas(
            &bubble_canvas,
            (destination_left * scale).round(),
            (destination_top * scale).round(),
            entrance_scale,
            entrance_opacity,
        );
    }

    /// Swift `BatteryGauge`: battery outline + fill, "NN%", metric tag, in a capsule
    /// (compact variant for side cards: smaller battery, smaller digits, no metric tag).
    fn paint_gauge(&mut self, canvas: &mut Canvas, center_x: f32, center_y: f32, value: Option<f64>, is_compact: bool) {
        let scale = self.scale_factor();
        let pill_height = if is_compact { GAUGE_COMPACT_PILL_HEIGHT } else { GAUGE_PILL_HEIGHT };
        let top = center_y - pill_height / 2.0;
        let percent_text = value.map(|percentage| format!("{}%", percentage.round() as i64)).unwrap_or("–".to_string());
        let metric_label = if is_compact { "" } else { self.config.gauge_metric.short_label() };
        let body_width = if is_compact { GAUGE_COMPACT_BODY_WIDTH } else { GAUGE_BODY_WIDTH };
        let body_height = if is_compact { GAUGE_COMPACT_BODY_HEIGHT } else { GAUGE_BODY_HEIGHT };
        let percent_font_size = if is_compact { GAUGE_COMPACT_PERCENT_FONT_SIZE } else { GAUGE_PERCENT_FONT_SIZE };

        self.label_font.set_size(percent_font_size, scale);
        let percent_width = self.label_font.measure_tabular_digits(&percent_text) / scale;
        self.label_font.set_size(GAUGE_METRIC_FONT_SIZE, scale);
        let metric_width = if metric_label.is_empty() { 0.0 } else { self.label_font.measure(metric_label) / scale };

        let battery_width = body_width + 1.0 + GAUGE_TIP_WIDTH;
        let mut pill_width = 2.0 * GAUGE_HORIZONTAL_PADDING + battery_width + GAUGE_ITEM_SPACING + percent_width;
        if metric_width > 0.0 {
            pill_width += GAUGE_ITEM_SPACING + metric_width;
        }
        let pill_left = center_x - pill_width / 2.0;
        let pill_center_y = top + pill_height / 2.0;
        canvas.fill_rounded_rect(
            pill_left * scale,
            top * scale,
            pill_width * scale,
            pill_height * scale,
            pill_height / 2.0 * scale,
            self.theme.window_background.with_opacity(0.85),
        );
        canvas.stroke_rounded_rect(
            pill_left * scale,
            top * scale,
            pill_width * scale,
            pill_height * scale,
            pill_height / 2.0 * scale,
            1.0 * scale,
            self.theme.primary.with_opacity(0.10),
        );

        // Battery body, fill and tip.
        let body_left = pill_left + GAUGE_HORIZONTAL_PADDING;
        let body_top = pill_center_y - body_height / 2.0;
        canvas.stroke_rounded_rect(
            body_left * scale,
            body_top * scale,
            body_width * scale,
            body_height * scale,
            2.0 * scale,
            1.0 * scale,
            self.theme.primary.with_opacity(0.45),
        );
        let fill_color = match value {
            None => self.theme.gray,
            Some(remaining) if remaining <= 15.0 => self.theme.red,
            Some(remaining) if remaining <= 40.0 => self.theme.yellow,
            Some(_) => self.theme.green,
        };
        let fraction = (value.unwrap_or(0.0).clamp(0.0, 100.0) / 100.0) as f32;
        let fill_width = ((body_width - 4.0) * fraction).max(0.0);
        if fill_width > 0.0 {
            canvas.fill_rounded_rect(
                (body_left + 2.0) * scale,
                (body_top + 2.0) * scale,
                fill_width * scale,
                (body_height - 4.0) * scale,
                1.0 * scale,
                fill_color,
            );
        }
        let tip_height = body_height * 0.45;
        canvas.fill_rounded_rect(
            (body_left + body_width + 1.0) * scale,
            (pill_center_y - tip_height / 2.0) * scale,
            GAUGE_TIP_WIDTH * scale,
            tip_height * scale,
            0.5 * scale,
            self.theme.primary.with_opacity(0.45),
        );

        // "NN%" then the metric tag.
        let mut cursor_x = body_left + battery_width + GAUGE_ITEM_SPACING;
        self.label_font.set_size(percent_font_size, scale);
        let percent_color = if value.is_none() { self.theme.tertiary() } else { self.theme.secondary() };
        let percent_top = pill_center_y * scale - self.label_font.line_height() / 2.0;
        canvas.draw_text_tabular_digits(&mut self.label_font, &percent_text, cursor_x * scale, percent_top, percent_color);
        cursor_x += percent_width + GAUGE_ITEM_SPACING;
        if metric_width > 0.0 {
            self.label_font.set_size(GAUGE_METRIC_FONT_SIZE, scale);
            let metric_top = pill_center_y * scale - self.label_font.line_height() / 2.0;
            canvas.draw_text(&mut self.label_font, metric_label, cursor_x * scale, metric_top, self.theme.tertiary());
        }
    }

    /// Swift `SessionBadge`: [red dot] text [status badge] in a capsule; the primary card's
    /// capsule is stroked in the accent colour while the row is expanded.
    #[allow(clippy::too_many_arguments)]
    fn paint_session_label(
        &mut self,
        canvas: &mut Canvas,
        card_id: &str,
        full_text: &str,
        state: PetState,
        is_dimmed: bool,
        is_highlighted: bool,
        has_attention_dot: bool,
        center_x: f32,
        center_y: f32,
        now: f64,
    ) {
        let scale = self.scale_factor();
        let has_badge = !matches!(state, PetState::Idle | PetState::Thinking);
        let badge_diameter = STATUS_BADGE_ICON_FRAME + 2.0 * STATUS_BADGE_PADDING;

        self.label_font.set_size(SESSION_LABEL_FONT_SIZE, scale);
        let text_line_height = SESSION_LABEL_TEXT_LINE_HEIGHT;
        let trailing_padding =
            if has_badge { SESSION_LABEL_TRAILING_PADDING_WITH_BADGE } else { SESSION_LABEL_TRAILING_PADDING };
        let badge_slot = if has_badge { SESSION_LABEL_ITEM_SPACING + badge_diameter } else { 0.0 };
        let dot_slot = if has_attention_dot { ATTENTION_DOT_DIAMETER + SESSION_LABEL_ITEM_SPACING } else { 0.0 };
        let text_width_limit =
            SESSION_LABEL_MAX_WIDTH - SESSION_LABEL_LEADING_PADDING - trailing_padding - badge_slot - dot_slot;
        let font = &mut self.label_font;
        let text = draw::wrap_text(full_text, text_width_limit * scale, 1, |candidate| font.measure(candidate))
            .into_iter()
            .next()
            .unwrap_or_default();
        let text_width = self.label_font.measure(&text) / scale;
        let capsule_width = SESSION_LABEL_LEADING_PADDING + dot_slot + text_width + badge_slot + trailing_padding;
        let capsule_height = text_line_height.max(if has_badge { badge_diameter } else { 0.0 })
            + 2.0 * SESSION_LABEL_VERTICAL_PADDING;
        let capsule_left = center_x - capsule_width / 2.0;
        let capsule_top = center_y - capsule_height / 2.0;

        canvas.fill_rounded_rect(
            capsule_left * scale,
            capsule_top * scale,
            capsule_width * scale,
            capsule_height * scale,
            capsule_height / 2.0 * scale,
            self.theme.window_background.with_opacity(0.85),
        );
        let (stroke_color, stroke_width) = if is_highlighted {
            (self.theme.accent.with_opacity(0.7), 1.2)
        } else {
            (self.theme.primary.with_opacity(0.10), 1.0)
        };
        canvas.stroke_rounded_rect(
            capsule_left * scale,
            capsule_top * scale,
            capsule_width * scale,
            capsule_height * scale,
            capsule_height / 2.0 * scale,
            stroke_width * scale,
            stroke_color,
        );
        let mut cursor_x = capsule_left + SESSION_LABEL_LEADING_PADDING;
        if has_attention_dot {
            canvas.fill_circle(
                (cursor_x + ATTENTION_DOT_DIAMETER / 2.0) * scale,
                center_y * scale,
                ATTENTION_DOT_DIAMETER / 2.0 * scale,
                self.theme.red,
            );
            cursor_x += dot_slot;
        }
        let text_color = if is_dimmed { self.theme.tertiary() } else { self.theme.secondary() };
        let text_top = center_y * scale - self.label_font.line_height() / 2.0;
        canvas.draw_text(&mut self.label_font, &text, cursor_x * scale, text_top, text_color);
        cursor_x += text_width;

        // Badge pop-in: remember when this card's badge state last changed.
        let changed_at = match self.badge_state_by_card.get(card_id) {
            Some((previous_state, changed_at)) if *previous_state == state => *changed_at,
            _ => {
                self.badge_state_by_card.insert(card_id.to_string(), (state, now));
                now
            }
        };
        if has_badge {
            let (pop_scale, pop_opacity) = animation::badge_appearance((now - changed_at) as f32);
            let badge_center_x = cursor_x + SESSION_LABEL_ITEM_SPACING + badge_diameter / 2.0;
            self.paint_status_badge(
                canvas,
                state,
                badge_center_x * scale,
                center_y * scale,
                badge_diameter / 2.0 * scale * pop_scale,
                pop_opacity,
                now,
            );
        }
    }

    /// Swift `StatusBadge` (compact): a tinted disc with a white ring and a white symbol
    /// (clock / ? / check / ! / gear / wave); the clock and ? pulse, the gear spins.
    #[allow(clippy::too_many_arguments)]
    fn paint_status_badge(
        &mut self,
        canvas: &mut Canvas,
        state: PetState,
        center_x: f32,
        center_y: f32,
        radius: f32,
        opacity: f32,
        now: f64,
    ) {
        if radius <= 0.0 || opacity <= 0.0 {
            return;
        }
        let scale = self.scale_factor();
        let (tint, symbol, is_pulsing) = match state {
            PetState::WaitingApproval => (self.theme.red, BadgeSymbol::Clock, true),
            PetState::NeedsInput => (self.theme.orange, BadgeSymbol::Glyph("?"), true),
            PetState::Done => (self.theme.green, BadgeSymbol::Check, false),
            PetState::Error => (self.theme.red, BadgeSymbol::Glyph("!"), false),
            PetState::Working => (self.theme.blue, BadgeSymbol::Gear, false),
            PetState::Hello => (self.theme.yellow, BadgeSymbol::Wave, false),
            PetState::Idle | PetState::Thinking => return,
        };
        let tint = tint.with_opacity(opacity);
        // Effects are phased on wall-clock seconds; the modulo keeps f32 precision.
        let effect_clock = (now % 1000.0) as f32;
        let symbol_opacity = if is_pulsing { animation::badge_pulse_opacity(effect_clock) } else { 1.0 } * opacity;
        let full_radius = (STATUS_BADGE_ICON_FRAME / 2.0 + STATUS_BADGE_PADDING) * scale;
        let symbol_scale = (radius / full_radius).clamp(0.1, 1.0);
        let white = Color::WHITE.with_opacity(symbol_opacity);
        canvas.fill_circle(center_x + 0.5 * scale, center_y + 1.0 * scale, radius, Color::rgba(0, 0, 0, 40).with_opacity(opacity));
        canvas.fill_circle(center_x, center_y, radius, tint);
        canvas.stroke_circle(center_x, center_y, radius, STATUS_BADGE_RING_WIDTH * scale, Color::WHITE.with_opacity(0.9 * opacity));
        let stroke = (1.6 * scale).max(1.0);
        let inner_radius = radius - STATUS_BADGE_RING_WIDTH * scale;
        match symbol {
            BadgeSymbol::Clock => {
                canvas.draw_line(center_x, center_y, center_x, center_y - inner_radius * 0.62, stroke, white);
                canvas.draw_line(center_x, center_y, center_x + inner_radius * 0.45, center_y, stroke, white);
            }
            BadgeSymbol::Check => {
                let unit = inner_radius * 0.5;
                canvas.draw_line(center_x - unit, center_y, center_x - unit * 0.25, center_y + unit * 0.75, stroke, white);
                canvas.draw_line(center_x - unit * 0.25, center_y + unit * 0.75, center_x + unit, center_y - unit * 0.7, stroke, white);
            }
            BadgeSymbol::Gear => {
                // `gearshape.fill`: a filled disc with eight stubby teeth and a centre hole.
                let hub_radius = inner_radius * 0.66;
                let tooth_thickness = (inner_radius * 0.5).max(1.0);
                let rotation = animation::gear_rotation_radians(effect_clock);
                for tooth in 0..8 {
                    let angle = tooth as f32 * std::f32::consts::FRAC_PI_4 + rotation;
                    let (sine, cosine) = angle.sin_cos();
                    canvas.draw_line(
                        center_x + cosine * hub_radius * 0.8,
                        center_y + sine * hub_radius * 0.8,
                        center_x + cosine * inner_radius * 0.8,
                        center_y + sine * inner_radius * 0.8,
                        tooth_thickness,
                        white,
                    );
                }
                canvas.fill_circle(center_x, center_y, hub_radius, white);
                canvas.fill_circle(center_x, center_y, inner_radius * 0.3, tint);
            }
            BadgeSymbol::Wave => {
                let unit = inner_radius * 0.55;
                canvas.draw_line(center_x - unit, center_y + unit * 0.2, center_x - unit * 0.35, center_y - unit * 0.4, stroke, white);
                canvas.draw_line(center_x - unit * 0.35, center_y - unit * 0.4, center_x + unit * 0.35, center_y + unit * 0.4, stroke, white);
                canvas.draw_line(center_x + unit * 0.35, center_y + unit * 0.4, center_x + unit, center_y - unit * 0.2, stroke, white);
            }
            BadgeSymbol::Glyph(glyph) => {
                self.badge_font.set_size(STATUS_BADGE_ICON_SIZE * symbol_scale, scale);
                let glyph_width = self.badge_font.measure(glyph);
                let glyph_top = center_y - self.badge_font.line_height() / 2.0;
                canvas.draw_text(&mut self.badge_font, glyph, center_x - glyph_width / 2.0, glyph_top, white);
            }
        }
    }

    // MARK: - Input

    /// Content x of the cursor in points (from the panel's left edge).
    fn cursor_content_x(&self) -> f32 {
        (self.cursor.0 / self.scale_factor() as f64) as f32
    }

    fn handle_mouse_down(&mut self) {
        if self.modifiers.control_key() {
            // Control-click is the keyboard-only way to get a context menu; treat it like right-click.
            self.mouse_down_position = None;
            self.show_context_menu();
            return;
        }
        self.mouse_down_position = Some(self.cursor);
    }

    fn handle_cursor_moved(&mut self) {
        let Some((down_x, down_y)) = self.mouse_down_position else { return };
        let threshold = DRAG_THRESHOLD_POINTS * self.scale_factor() as f64;
        if (self.cursor.0 - down_x).abs() > threshold || (self.cursor.1 - down_y).abs() > threshold {
            // A drag: hand the rest of the gesture to AppKit (the release never reaches us).
            self.mouse_down_position = None;
            if let Some(window) = &self.window {
                let _ = window.drag_window();
            }
        }
    }

    fn handle_mouse_up(&mut self) {
        if self.mouse_down_position.take().is_some() {
            let content_x = self.cursor_content_x();
            super::log(&format!(
                "mouseUp view=({},{}) bounds={}x{}",
                content_x as i64,
                (self.cursor.1 / self.scale_factor() as f64) as i64,
                self.model.layout.content_width as i64,
                self.model.layout.content_height as i64
            ));
            self.handle_click(content_x);
        }
    }

    /// A click at `content_x` (points from the panel's left edge) — from the mouse or from
    /// `claude-airou click`. Logs the outcome like the Swift view model.
    fn handle_click(&mut self, content_x: f32) {
        let phrases = self.pet.pet_phrases();
        let now = now_epoch_secs();
        let cards = self.model.cards_description();
        let width = self.model.layout.content_width as i64;
        let session_count = self.model.sessions.len();
        let expanded = self.model.is_expanded();
        let action = self.model.handle_click(content_x, &phrases, entropy_seed(), now);
        let description = match &action {
            ClickAction::Pet => "pet".to_string(),
            ClickAction::Expand => "expand".to_string(),
            ClickAction::Collapse => "collapse".to_string(),
            ClickAction::Pin(session_id) => format!("pin {session_id}"),
            ClickAction::Ignored => "ignored".to_string(),
        };
        super::log(&format!(
            "click x={} width={width} sessions={session_count} expanded={expanded} cards: {cards} -> {description}",
            content_x as i64
        ));
        self.relayout();
        self.request_redraw();
        self.schedule_animation_frames();
    }
}

enum BadgeSymbol {
    Clock,
    Check,
    Gear,
    Wave,
    Glyph(&'static str),
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        self.create_window(event_loop);
        if self.window.is_some() {
            // The tray must be created while the event loop runs (tray-icon on macOS).
            self.create_tray();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.quit(event_loop),
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x, position.y);
                self.handle_cursor_moved();
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                self.handle_mouse_down()
            }
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Left, .. } => {
                self.handle_mouse_up()
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Right, .. } => {
                self.show_context_menu()
            }
            WindowEvent::Moved(_) => self.remember_origin(),
            WindowEvent::ScaleFactorChanged { .. } => {
                self.relayout();
                self.request_redraw();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if event_loop.exiting() {
            return;
        }
        let now = Instant::now();
        if now >= self.next_tick {
            self.tick(event_loop);
            while self.next_tick <= now {
                self.next_tick += TICK;
            }
        }
        if let Some(next_frame) = self.next_animation_frame {
            if now >= next_frame {
                self.request_redraw();
                self.next_animation_frame = if self.model.is_animating(now_epoch_secs()) {
                    Some(now + ANIMATION_FRAME)
                } else {
                    None
                };
            }
        }
        if !event_loop.exiting() {
            let wake = match self.next_animation_frame {
                Some(next_frame) => self.next_tick.min(next_frame),
                None => self.next_tick,
            };
            event_loop.set_control_flow(ControlFlow::WaitUntil(wake));
        }
    }
}

/// `NSWorkspace.shared.open` equivalent for a folder or a log file.
fn open_with_finder(path: &std::path::Path) {
    if let Err(error) = std::process::Command::new("open").arg(path).spawn() {
        super::log(&format!("open {} failed: {error}", path.display()));
    }
}

/// Nanosecond clock entropy for picking a random pet phrase (no rand dependency).
fn entropy_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos() as u64 ^ duration.as_secs())
        .unwrap_or(0)
}
