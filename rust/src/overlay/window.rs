//! The overlay window and event loop (winit 0.30 + CALayer presenter + tray-icon).
//!
//! The window is a transparent, shadowless, always-on-top winit window whose NSView
//! layer shows the software canvas (`present_macos.rs`), so only the drawn pixels are
//! visible: the pet sprite, the speech bubble, the battery gauge pill and the session
//! label capsule float over the desktop exactly like the Swift `PetView`.
//!
//! Layout constants below are the Swift `RowLayout` / `PetView` values in points; every
//! paint call converts to physical pixels through the window's scale factor.
//! Platform-specific tweaks (transparency, Spaces behaviour, layer presenting) live in
//! `present_macos.rs`; this file is meant to stay portable.

use std::rc::Rc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS, WindowAttributesExtMacOS};
use winit::window::{Window, WindowId, WindowLevel};

use super::draw::{self, Canvas, Color};
use super::present_macos::{is_dark_appearance, LayerPresenter};
use super::text::{FontStyle, TextRasterizer};
use super::{lock, logic::OverlayModel, tray};
use crate::model::{now_epoch_secs, AppConfig, GaugeMetric, PetState};
use crate::pets::{PetDefinition, PetLibrary, ResolvedPalette};
use crate::state_store::StateStore;

/// Poll cadence for session state, menu events and animation (spec: 0.3 s).
const TICK: Duration = Duration::from_millis(300);
/// Debounced config save after window moves (Swift: 0.6 s).
const CONFIG_SAVE_DEBOUNCE: Duration = Duration::from_millis(600);
/// Refresh the pid lock's mtime roughly every 18 s (60 ticks) so the one-day
/// staleness heuristic keeps other instances out while we are alive.
const LOCK_TOUCH_EVERY_TICKS: u64 = 60;

// Swift RowLayout / PetView geometry, in points.
const MINIMUM_CONTENT_WIDTH: f32 = 220.0;
const HORIZONTAL_PADDING: f32 = 16.0;
const MINIMUM_CARD_WIDTH: f32 = 72.0;
const MAXIMUM_CARD_WIDTH: f32 = 132.0;
const SPEECH_BUBBLE_RESERVED_HEIGHT: f32 = 66.0;
const SPEECH_BUBBLE_BOTTOM_INSET: f32 = 12.0;
const SPEECH_BUBBLE_MAX_WIDTH: f32 = 300.0;
const SPEECH_BUBBLE_MIN_WIDTH: f32 = 40.0;
const SPEECH_BUBBLE_EDGE_MARGIN: f32 = 4.0;
const SPEECH_BUBBLE_HORIZONTAL_PADDING: f32 = 9.0;
const SPEECH_BUBBLE_VERTICAL_PADDING: f32 = 6.0;
const SPEECH_BUBBLE_CORNER_RADIUS: f32 = 9.0;
const SPEECH_BUBBLE_TAIL_WIDTH: f32 = 12.0;
const SPEECH_BUBBLE_TAIL_HEIGHT: f32 = 6.0;
const SPEECH_BUBBLE_FONT_SIZE: f32 = 11.5;
const SPEECH_BUBBLE_MAX_LINES: usize = 2;
const SESSION_BADGE_RESERVED_HEIGHT: f32 = 22.0;
const SESSION_LABEL_FONT_SIZE: f32 = 9.5;
const SESSION_LABEL_LEADING_PADDING: f32 = 7.0;
const SESSION_LABEL_TRAILING_PADDING: f32 = 7.0;
const SESSION_LABEL_TRAILING_PADDING_WITH_BADGE: f32 = 4.0;
const SESSION_LABEL_VERTICAL_PADDING: f32 = 2.5;
const SESSION_LABEL_BADGE_SPACING: f32 = 4.0;
const SESSION_LABEL_MAX_WIDTH: f32 = 200.0;
const STATUS_BADGE_ICON_SIZE: f32 = 10.0;
const STATUS_BADGE_PADDING: f32 = 2.0;
const STATUS_BADGE_RING_WIDTH: f32 = 1.5;
const GAUGE_RESERVED_HEIGHT: f32 = 16.0;
const GAUGE_BODY_WIDTH: f32 = 24.0;
const GAUGE_BODY_HEIGHT: f32 = 10.0;
const GAUGE_TIP_WIDTH: f32 = 1.5;
const GAUGE_PERCENT_FONT_SIZE: f32 = 9.0;
const GAUGE_METRIC_FONT_SIZE: f32 = 7.5;
const GAUGE_HORIZONTAL_PADDING: f32 = 5.0;
const GAUGE_ITEM_SPACING: f32 = 3.0;
const CARD_VERTICAL_SPACING: f32 = 4.0;
const VERTICAL_PADDING: f32 = 12.0;

// System colours (macOS dark-appearance values).
const RED: Color = Color::rgb(255, 69, 58);
const ORANGE: Color = Color::rgb(255, 159, 10);
const GREEN: Color = Color::rgb(48, 209, 88);
const BLUE: Color = Color::rgb(10, 132, 255);
const YELLOW: Color = Color::rgb(255, 214, 10);
const GRAY: Color = Color::rgb(152, 152, 157);

/// The appearance-dependent palette (`windowBackgroundColor`, `.primary`, `.secondary`).
#[derive(Clone, Copy)]
struct Theme {
    window_background: Color,
    primary: Color,
}

impl Theme {
    fn for_appearance(is_dark: bool) -> Theme {
        if is_dark {
            Theme { window_background: Color::rgb(50, 50, 50), primary: Color::WHITE }
        } else {
            Theme { window_background: Color::rgb(236, 236, 236), primary: Color::rgb(0, 0, 0) }
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
    let mut app = App {
        config,
        library,
        pet,
        palette,
        model: OverlayModel::new(),
        store: StateStore::default(),
        window: None,
        presenter: None,
        bubble_font: TextRasterizer::load_system(FontStyle::BUBBLE),
        label_font: TextRasterizer::load_system(FontStyle::LABEL),
        badge_font: TextRasterizer::load_system(FontStyle::BADGE),
        theme: Theme::for_appearance(true),
        tray_icon: None,
        menu_model: None,
        next_tick: Instant::now() + TICK,
        tick_count: 0,
        cursor: (0.0, 0.0),
        pet_rect: (0, 0, 0, 0),
        last_requested_size: None,
        config_dirty_at: None,
        lock_path: crate::paths::overlay_lock_file(),
        pid: std::process::id(),
        exit_code: 0,
    };

    match event_loop.run_app(&mut app) {
        Ok(()) => app.exit_code,
        Err(error) => {
            crate::logging::eprint_line(&format!("claude-airou: overlay event loop failed: {error}"));
            1
        }
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
    bubble_font: TextRasterizer,
    label_font: TextRasterizer,
    badge_font: TextRasterizer,
    theme: Theme,
    tray_icon: Option<tray_icon::TrayIcon>,
    menu_model: Option<tray::MenuModel>,
    next_tick: Instant,
    tick_count: u64,
    cursor: (f64, f64),
    /// Pet sprite hit-box in physical pixels: (x, y, w, h), updated on every redraw.
    pet_rect: (i32, i32, i32, i32),
    /// The inner size we last asked winit for; resizes are shifted symmetrically only
    /// relative to a size we set ourselves (never relative to winit's default size).
    last_requested_size: Option<PhysicalSize<u32>>,
    config_dirty_at: Option<Instant>,
    lock_path: std::path::PathBuf,
    pid: u32,
    exit_code: i32,
}

/// Where everything goes, in points (the Swift `RowLayout` for one card).
struct Layout {
    content_width: f32,
    content_height: f32,
    /// Width the window has with no bubble (the persisted origin is relative to this).
    collapsed_width: f32,
    sprite_width: f32,
    sprite_height: f32,
    shows_gauge: bool,
    bubble_lines: Vec<String>,
    /// Bubble box width in points (0 when hidden).
    bubble_width: f32,
}

impl App {
    // MARK: - Layout

    fn scale_factor(&self) -> f32 {
        self.window.as_ref().map(|window| window.scale_factor()).unwrap_or(1.0) as f32
    }

    /// Physical pixels per pet pixel (whole number so the sprite stays crisp).
    fn sprite_pixel_scale(&self) -> u32 {
        ((self.config.pixel_scale as f32 * self.scale_factor()).round() as i64).max(1) as u32
    }

    fn grid_size(&self) -> (u32, u32) {
        let (width, height) = self.pet.grid_size();
        (width.max(1) as u32, height.max(1) as u32)
    }

    fn shows_gauge(&self) -> bool {
        self.config.gauge_metric != GaugeMetric::Off
    }

    fn is_bubble_visible(&self) -> bool {
        self.model.is_speech_bubble_visible(self.config.is_speech_bubble_hidden)
    }

    fn compute_layout(&mut self) -> Layout {
        let scale = self.scale_factor();
        let (grid_width, grid_height) = self.grid_size();
        let sprite_pixel_scale = self.sprite_pixel_scale();
        let sprite_width = (grid_width * sprite_pixel_scale) as f32 / scale;
        let sprite_height = (grid_height * sprite_pixel_scale) as f32 / scale;
        let shows_gauge = self.shows_gauge();

        let card_width = sprite_width.max(MINIMUM_CARD_WIDTH).min(MAXIMUM_CARD_WIDTH);
        let collapsed_width = MINIMUM_CONTENT_WIDTH.max(card_width + 2.0 * HORIZONTAL_PADDING);

        // Speech bubble: single line up to the max width, then wrapped to two lines.
        let (bubble_lines, bubble_width) = if self.is_bubble_visible() {
            self.bubble_font.set_pixel_size(SPEECH_BUBBLE_FONT_SIZE * scale);
            let text = self.model.speech_text().to_string();
            let single_line_width = self.bubble_font.measure(&text) / scale;
            let bubble_width = (single_line_width + 2.0 * SPEECH_BUBBLE_HORIZONTAL_PADDING + 2.0)
                .ceil()
                .clamp(SPEECH_BUBBLE_MIN_WIDTH, SPEECH_BUBBLE_MAX_WIDTH);
            let text_width_limit = (bubble_width - 2.0 * SPEECH_BUBBLE_HORIZONTAL_PADDING) * scale;
            let font = &mut self.bubble_font;
            let lines = draw::wrap_text(&text, text_width_limit, SPEECH_BUBBLE_MAX_LINES, |candidate| {
                font.measure(candidate)
            });
            (lines, bubble_width)
        } else {
            (Vec::new(), 0.0)
        };
        let content_width = collapsed_width.max(bubble_width + 2.0 * SPEECH_BUBBLE_EDGE_MARGIN);
        let gauge_height = if shows_gauge { GAUGE_RESERVED_HEIGHT } else { 0.0 };
        let content_height = SPEECH_BUBBLE_RESERVED_HEIGHT
            + sprite_height
            + gauge_height
            + SESSION_BADGE_RESERVED_HEIGHT
            + VERTICAL_PADDING;
        Layout {
            content_width,
            content_height,
            collapsed_width,
            sprite_width,
            sprite_height,
            shows_gauge,
            bubble_lines,
            bubble_width,
        }
    }

    fn desired_size(&self, layout: &Layout) -> PhysicalSize<u32> {
        let scale = self.scale_factor();
        PhysicalSize::new(
            ((layout.content_width * scale).round() as u32).max(1),
            ((layout.content_height * scale).round() as u32).max(1),
        )
    }

    /// Resizes the window to fit the layout, keeping the pet where it is on screen
    /// (the width grows symmetrically around the centred card, like the Swift panel).
    fn sync_window_size(&mut self) {
        let layout = self.compute_layout();
        let desired = self.desired_size(&layout);
        let Some(window) = self.window.clone() else { return };
        let current = window.inner_size();
        if current != desired {
            let is_known_size = self.last_requested_size == Some(current);
            if is_known_size {
                if let Ok(position) = window.outer_position() {
                    let shift_x = (current.width as i32 - desired.width as i32) / 2;
                    if shift_x != 0 {
                        window.set_outer_position(PhysicalPosition::new(position.x + shift_x, position.y));
                    }
                }
            }
            let _ = window.request_inner_size(desired);
            self.last_requested_size = Some(desired);
        }
        window.request_redraw();
    }

    /// Places the window so that its *collapsed* (no-bubble) origin is `collapsed_origin`
    /// in logical coordinates — the inverse of `remember_origin`.
    fn place_at_collapsed_origin(&mut self, collapsed_origin: LogicalPosition<f64>) {
        let scale = self.scale_factor();
        let layout = self.compute_layout();
        let Some(window) = self.window.clone() else { return };
        let extra_width = layout.content_width - layout.collapsed_width;
        let x = ((collapsed_origin.x as f32 - extra_width / 2.0) * scale).round() as i32;
        let y = (collapsed_origin.y as f32 * scale).round() as i32;
        window.set_outer_position(PhysicalPosition::new(x, y));
    }

    // MARK: - Startup

    fn create_window(&mut self, event_loop: &ActiveEventLoop) {
        let saved_origin = match (self.config.window_origin_x, self.config.window_origin_y) {
            (Some(x), Some(y)) => Some(LogicalPosition::new(x, y)),
            _ => None,
        };
        let initial_layout = self.compute_layout();
        let mut attributes = Window::default_attributes()
            .with_title("Claude Airou")
            .with_inner_size(LogicalSize::new(initial_layout.content_width, initial_layout.content_height))
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_visible(!self.config.is_pet_hidden)
            .with_has_shadow(false)
            .with_accepts_first_mouse(true);
        if let Some(origin) = saved_origin {
            attributes = attributes.with_position(origin);
        }
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
        match LayerPresenter::attach(&window) {
            Some(presenter) => self.presenter = Some(presenter),
            None => {
                crate::logging::eprint_line("claude-airou: could not attach the overlay's content layer");
                self.exit_code = 1;
                event_loop.exit();
                return;
            }
        }
        if self.config.is_click_through {
            let _ = window.set_cursor_hittest(false);
        }
        self.window = Some(window);
        self.theme = Theme::for_appearance(is_dark_appearance());
        self.sync_window_size();
        match saved_origin {
            Some(origin) => self.place_at_collapsed_origin(origin),
            None => self.reset_position(),
        }
        super::log(&format!("overlay started (pid {}, pet {})", self.pid, self.pet.id));
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

    // MARK: - Tray menu

    fn current_menu_model(&self) -> tray::MenuModel {
        let selected_id = self.pet.id.as_str();
        tray::MenuModel {
            header: self.model.menu_header(&self.pet.name),
            usage_line: self
                .model
                .usage_summary(self.model.focused.as_ref().map(|session| session.session_id.as_str())),
            pets: tray::MenuModel::pet_rows(&self.library, selected_id),
            pixel_scale: self.config.pixel_scale,
            gauge_metric: self.config.gauge_metric,
            bubbles_hidden: self.config.is_speech_bubble_hidden,
            click_through: self.config.is_click_through,
            pet_hidden: self.config.is_pet_hidden,
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

    fn handle_menu_action(&mut self, action: tray::MenuAction, event_loop: &ActiveEventLoop) {
        use tray::MenuAction::*;
        super::log(&format!("menu: {}", tray::menu_id_for(&action)));
        match action {
            SelectPet(pet_id) => {
                if let Some(loaded) = self.library.pet_with_id(&pet_id) {
                    self.pet = loaded.definition.clone();
                    self.palette = ResolvedPalette::new(&self.pet);
                    self.model.frame_index = 0;
                    self.config.selected_pet_id = Some(pet_id);
                    self.config.save();
                    self.sync_window_size();
                }
            }
            SelectSize(scale) => {
                self.config.pixel_scale = scale;
                self.config.save();
                self.sync_window_size();
            }
            SelectGauge(metric) => {
                self.config.gauge_metric = metric;
                self.config.save();
                self.sync_window_size();
            }
            ToggleBubbles => {
                self.config.is_speech_bubble_hidden = !self.config.is_speech_bubble_hidden;
                self.config.save();
                self.sync_window_size();
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
            ResetPosition => self.reset_position(),
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
            self.sync_window_size();
        }
    }

    /// Bottom-right corner of the current monitor with a 24-point margin (Swift uses
    /// the screen's visibleFrame; winit has no dock/menu-bar-aware equivalent, so this
    /// uses the full monitor bounds — close enough for "Reset position").
    fn reset_position(&mut self) {
        let Some(window) = &self.window else { return };
        let Some(monitor) = window.current_monitor().or_else(|| window.primary_monitor()) else {
            return;
        };
        let monitor_position = monitor.position();
        let monitor_size = monitor.size();
        let window_size = window.outer_size();
        let margin = (24.0 * self.scale_factor()).round() as i32;
        let x = monitor_position.x + monitor_size.width as i32 - window_size.width as i32 - margin;
        let y = monitor_position.y + monitor_size.height as i32 - window_size.height as i32 - margin;
        window.set_outer_position(PhysicalPosition::new(x, y));
        self.remember_origin(PhysicalPosition::new(x, y));
    }

    /// Persists the window origin as it would be with no speech bubble (the bubble only
    /// widens the window symmetrically), so a wide bubble at quit time cannot shift the
    /// pet on the next launch.
    fn remember_origin(&mut self, position: PhysicalPosition<i32>) {
        let scale = self.scale_factor();
        let current_width = self.window.as_ref().map(|window| window.inner_size().width).unwrap_or(0) as f32;
        let collapsed_width = self.compute_layout().collapsed_width * scale;
        let collapsed_x = position.x as f32 + (current_width - collapsed_width) / 2.0;
        self.config.window_origin_x = Some((collapsed_x / scale) as f64);
        self.config.window_origin_y = Some((position.y as f32 / scale) as f64);
        self.config_dirty_at = Some(Instant::now());
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

        let sessions = self.store.load_all();
        let usage = self.store.load_all_usage();
        self.model.reload(sessions, usage);
        self.model.advance_frames(TICK.as_secs_f64(), self.pet.frames_per_second());
        self.model.expire_pet_reaction_if_due(now_epoch_secs());

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

        if self.tick_count % LOCK_TOUCH_EVERY_TICKS == 0 {
            lock::touch(&self.lock_path, self.pid);
            self.theme = Theme::for_appearance(is_dark_appearance());
        }
        if let Some(dirty_at) = self.config_dirty_at {
            if dirty_at.elapsed() >= CONFIG_SAVE_DEBOUNCE {
                self.config_dirty_at = None;
                self.config.save();
            }
        }

        self.sync_tray_menu();
        // The bubble text drives the window width, so re-layout every tick.
        self.sync_window_size();
    }

    // MARK: - Drawing

    fn redraw(&mut self) {
        let Some(window) = self.window.clone() else { return };
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        let layout = self.compute_layout();
        let mut canvas = Canvas::new(size.width, size.height);
        self.paint(&mut canvas, &layout);
        if let Some(presenter) = &self.presenter {
            presenter.present(&canvas, window.scale_factor());
        }
    }

    fn paint(&mut self, canvas: &mut Canvas, layout: &Layout) {
        let scale = self.scale_factor();
        let content_width = canvas.width as f32 / scale;
        let card_center_x = content_width / 2.0;

        // Speech bubble, bottom-aligned above the card and centred over the pet.
        if !layout.bubble_lines.is_empty() {
            self.paint_speech_bubble(canvas, layout, card_center_x);
        }

        // Pet sprite.
        let sprite_pixel_scale = self.sprite_pixel_scale();
        let sprite_left = ((card_center_x - layout.sprite_width / 2.0) * scale).round() as i32;
        let sprite_top = (SPEECH_BUBBLE_RESERVED_HEIGHT * scale).round() as i32;
        let sprite_width_px = (layout.sprite_width * scale).round() as i32;
        let sprite_height_px = (layout.sprite_height * scale).round() as i32;
        self.pet_rect = (sprite_left, sprite_top, sprite_width_px, sprite_height_px);
        let frames = self.pet.frames_for(self.model.display_state);
        if !frames.is_empty() {
            let frame = &frames[self.model.frame_index % frames.len()];
            if let Ok((rgba, image_width, image_height)) =
                crate::render::frame_rgba(frame, &self.palette, sprite_pixel_scale, None)
            {
                canvas.blit_rgba(&rgba, image_width, image_height, sprite_left, sprite_top);
            }
        }

        // Gauge pill under the sprite (its reserved slot is 16 pt; the pill is 12 pt tall).
        let mut next_top = SPEECH_BUBBLE_RESERVED_HEIGHT + layout.sprite_height + CARD_VERTICAL_SPACING;
        if layout.shows_gauge {
            let pill_height = GAUGE_RESERVED_HEIGHT - CARD_VERTICAL_SPACING;
            self.paint_gauge(canvas, card_center_x, next_top, pill_height);
            next_top += GAUGE_RESERVED_HEIGHT;
        }

        // Session label capsule with the compact status badge, centred in its slot.
        self.paint_session_label(canvas, card_center_x, next_top + SESSION_BADGE_RESERVED_HEIGHT / 2.0);
    }

    fn paint_speech_bubble(&mut self, canvas: &mut Canvas, layout: &Layout, card_center_x: f32) {
        let scale = self.scale_factor();
        let content_width = canvas.width as f32 / scale;
        self.bubble_font.set_pixel_size(SPEECH_BUBBLE_FONT_SIZE * scale);
        let line_height = self.bubble_font.line_height() / scale;
        let bubble_width = layout.bubble_width.min(content_width - 2.0 * SPEECH_BUBBLE_EDGE_MARGIN);
        let bubble_height = layout.bubble_lines.len() as f32 * line_height + 2.0 * SPEECH_BUBBLE_VERTICAL_PADDING;
        let half_width = bubble_width / 2.0;
        let bubble_center_x = card_center_x
            .max(half_width + SPEECH_BUBBLE_EDGE_MARGIN)
            .min(content_width - half_width - SPEECH_BUBBLE_EDGE_MARGIN);
        let bubble_left = bubble_center_x - half_width;
        let bubble_bottom = SPEECH_BUBBLE_RESERVED_HEIGHT - SPEECH_BUBBLE_BOTTOM_INSET;
        let bubble_top = bubble_bottom - bubble_height;

        let fill = self.theme.window_background.with_opacity(0.94);
        // Soft shadow (Swift: black 18 %, radius 3, y offset 1), approximated by one halo.
        canvas.fill_rounded_rect(
            (bubble_left - 1.0) * scale,
            (bubble_top + 0.5) * scale,
            (bubble_width + 2.0) * scale,
            (bubble_height + 2.0) * scale,
            (SPEECH_BUBBLE_CORNER_RADIUS + 1.0) * scale,
            Color::rgba(0, 0, 0, 24),
        );
        canvas.fill_rounded_rect(
            bubble_left * scale,
            bubble_top * scale,
            bubble_width * scale,
            bubble_height * scale,
            SPEECH_BUBBLE_CORNER_RADIUS * scale,
            fill,
        );
        canvas.stroke_rounded_rect(
            bubble_left * scale,
            bubble_top * scale,
            bubble_width * scale,
            bubble_height * scale,
            SPEECH_BUBBLE_CORNER_RADIUS * scale,
            1.0 * scale,
            self.theme.primary.with_opacity(0.12),
        );
        // Tail: 12×6 triangle hanging 5.5 pt below the bubble (overlaps 0.5 pt so it joins).
        canvas.fill_triangle_down(
            (bubble_center_x - SPEECH_BUBBLE_TAIL_WIDTH / 2.0) * scale,
            (bubble_bottom - 0.5) * scale,
            SPEECH_BUBBLE_TAIL_WIDTH * scale,
            SPEECH_BUBBLE_TAIL_HEIGHT * scale,
            fill,
        );

        let text_left = (bubble_left + SPEECH_BUBBLE_HORIZONTAL_PADDING) * scale;
        let mut text_top = (bubble_top + SPEECH_BUBBLE_VERTICAL_PADDING) * scale;
        for line in &layout.bubble_lines {
            canvas.draw_text(&mut self.bubble_font, line, text_left, text_top, self.theme.primary);
            text_top += line_height * scale;
        }
    }

    /// Swift `BatteryGauge`: battery outline + fill, "NN%", metric tag, in a capsule.
    fn paint_gauge(&mut self, canvas: &mut Canvas, card_center_x: f32, top: f32, pill_height: f32) {
        let scale = self.scale_factor();
        let value = self.model.gauge_value(self.config.gauge_metric);
        let percent_text = value.map(|percentage| format!("{}%", percentage.round() as i64)).unwrap_or("–".to_string());
        let metric_label = self.config.gauge_metric.short_label();

        self.label_font.set_pixel_size(GAUGE_PERCENT_FONT_SIZE * scale);
        let percent_width = self.label_font.measure(&percent_text) / scale;
        self.label_font.set_pixel_size(GAUGE_METRIC_FONT_SIZE * scale);
        let metric_width = if metric_label.is_empty() { 0.0 } else { self.label_font.measure(metric_label) / scale };

        let battery_width = GAUGE_BODY_WIDTH + 1.0 + GAUGE_TIP_WIDTH;
        let mut pill_width = 2.0 * GAUGE_HORIZONTAL_PADDING + battery_width + GAUGE_ITEM_SPACING + percent_width;
        if metric_width > 0.0 {
            pill_width += GAUGE_ITEM_SPACING + metric_width;
        }
        let pill_left = card_center_x - pill_width / 2.0;
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
        let body_top = pill_center_y - GAUGE_BODY_HEIGHT / 2.0;
        canvas.stroke_rounded_rect(
            body_left * scale,
            body_top * scale,
            GAUGE_BODY_WIDTH * scale,
            GAUGE_BODY_HEIGHT * scale,
            2.0 * scale,
            1.0 * scale,
            self.theme.primary.with_opacity(0.45),
        );
        let fill_color = match value {
            None => GRAY,
            Some(remaining) if remaining <= 15.0 => RED,
            Some(remaining) if remaining <= 40.0 => YELLOW,
            Some(_) => GREEN,
        };
        let fraction = (value.unwrap_or(0.0).clamp(0.0, 100.0) / 100.0) as f32;
        let fill_width = ((GAUGE_BODY_WIDTH - 4.0) * fraction).max(0.0);
        if fill_width > 0.0 {
            canvas.fill_rounded_rect(
                (body_left + 2.0) * scale,
                (body_top + 2.0) * scale,
                fill_width * scale,
                (GAUGE_BODY_HEIGHT - 4.0) * scale,
                1.0 * scale,
                fill_color,
            );
        }
        let tip_height = GAUGE_BODY_HEIGHT * 0.45;
        canvas.fill_rounded_rect(
            (body_left + GAUGE_BODY_WIDTH + 1.0) * scale,
            (pill_center_y - tip_height / 2.0) * scale,
            GAUGE_TIP_WIDTH * scale,
            tip_height * scale,
            0.5 * scale,
            self.theme.primary.with_opacity(0.45),
        );

        // "NN%" then the metric tag.
        let mut cursor_x = body_left + battery_width + GAUGE_ITEM_SPACING;
        self.label_font.set_pixel_size(GAUGE_PERCENT_FONT_SIZE * scale);
        let percent_color = if value.is_none() { self.theme.tertiary() } else { self.theme.secondary() };
        let percent_top = pill_center_y * scale - self.label_font.line_height() / 2.0;
        canvas.draw_text(&mut self.label_font, &percent_text, cursor_x * scale, percent_top, percent_color);
        cursor_x += percent_width + GAUGE_ITEM_SPACING;
        if metric_width > 0.0 {
            self.label_font.set_pixel_size(GAUGE_METRIC_FONT_SIZE * scale);
            let metric_top = pill_center_y * scale - self.label_font.line_height() / 2.0;
            canvas.draw_text(&mut self.label_font, metric_label, cursor_x * scale, metric_top, self.theme.tertiary());
        }
    }

    /// Swift `SessionBadge`: "project" / "project +N" / "no session" in a capsule, with the
    /// compact status badge (clock / ? / check / ! / gear / wave) after the text.
    fn paint_session_label(&mut self, canvas: &mut Canvas, card_center_x: f32, center_y: f32) {
        let scale = self.scale_factor();
        let state = self.model.display_state;
        let has_badge = !matches!(state, PetState::Idle | PetState::Thinking);
        let badge_diameter = STATUS_BADGE_ICON_SIZE + 2.0 * STATUS_BADGE_PADDING;
        let full_text = self.model.collapsed_label();

        self.label_font.set_pixel_size(SESSION_LABEL_FONT_SIZE * scale);
        let text_line_height = self.label_font.line_height() / scale;
        let trailing_padding =
            if has_badge { SESSION_LABEL_TRAILING_PADDING_WITH_BADGE } else { SESSION_LABEL_TRAILING_PADDING };
        let badge_slot = if has_badge { SESSION_LABEL_BADGE_SPACING + badge_diameter } else { 0.0 };
        let text_width_limit = SESSION_LABEL_MAX_WIDTH - SESSION_LABEL_LEADING_PADDING - trailing_padding - badge_slot;
        let font = &mut self.label_font;
        let text = draw::wrap_text(&full_text, text_width_limit * scale, 1, |candidate| font.measure(candidate))
            .into_iter()
            .next()
            .unwrap_or_default();
        let text_width = self.label_font.measure(&text) / scale;
        let capsule_width = SESSION_LABEL_LEADING_PADDING + text_width + badge_slot + trailing_padding;
        let capsule_height = text_line_height.max(if has_badge { badge_diameter } else { 0.0 })
            + 2.0 * SESSION_LABEL_VERTICAL_PADDING;
        let capsule_left = card_center_x - capsule_width / 2.0;
        let capsule_top = center_y - capsule_height / 2.0;

        canvas.fill_rounded_rect(
            capsule_left * scale,
            capsule_top * scale,
            capsule_width * scale,
            capsule_height * scale,
            capsule_height / 2.0 * scale,
            self.theme.window_background.with_opacity(0.85),
        );
        canvas.stroke_rounded_rect(
            capsule_left * scale,
            capsule_top * scale,
            capsule_width * scale,
            capsule_height * scale,
            capsule_height / 2.0 * scale,
            1.0 * scale,
            self.theme.primary.with_opacity(0.10),
        );
        let text_color = if self.model.focused.is_none() { self.theme.tertiary() } else { self.theme.secondary() };
        let text_top = center_y * scale - self.label_font.line_height() / 2.0;
        canvas.draw_text(
            &mut self.label_font,
            &text,
            (capsule_left + SESSION_LABEL_LEADING_PADDING) * scale,
            text_top,
            text_color,
        );
        if has_badge {
            let badge_center_x = capsule_left
                + SESSION_LABEL_LEADING_PADDING
                + text_width
                + SESSION_LABEL_BADGE_SPACING
                + badge_diameter / 2.0;
            self.paint_status_badge(canvas, state, badge_center_x * scale, center_y * scale, badge_diameter / 2.0 * scale);
        }
    }

    /// Swift `StatusBadge` (compact): a tinted disc with a white ring and a white symbol.
    fn paint_status_badge(&mut self, canvas: &mut Canvas, state: PetState, center_x: f32, center_y: f32, radius: f32) {
        let scale = self.scale_factor();
        let (tint, symbol) = match state {
            PetState::WaitingApproval => (RED, BadgeSymbol::Clock),
            PetState::NeedsInput => (ORANGE, BadgeSymbol::Glyph("?")),
            PetState::Done => (GREEN, BadgeSymbol::Check),
            PetState::Error => (RED, BadgeSymbol::Glyph("!")),
            PetState::Working => (BLUE, BadgeSymbol::Gear),
            PetState::Hello => (YELLOW, BadgeSymbol::Wave),
            PetState::Idle | PetState::Thinking => return,
        };
        canvas.fill_circle(center_x + 0.5 * scale, center_y + 1.0 * scale, radius, Color::rgba(0, 0, 0, 40));
        canvas.fill_circle(center_x, center_y, radius, tint);
        canvas.stroke_circle(center_x, center_y, radius, STATUS_BADGE_RING_WIDTH * scale, Color::WHITE.with_opacity(0.9));
        let stroke = (1.6 * scale).max(1.0);
        let inner_radius = radius - STATUS_BADGE_RING_WIDTH * scale;
        match symbol {
            BadgeSymbol::Clock => {
                canvas.draw_line(center_x, center_y, center_x, center_y - inner_radius * 0.62, stroke, Color::WHITE);
                canvas.draw_line(center_x, center_y, center_x + inner_radius * 0.45, center_y, stroke, Color::WHITE);
            }
            BadgeSymbol::Check => {
                let unit = inner_radius * 0.5;
                canvas.draw_line(center_x - unit, center_y, center_x - unit * 0.25, center_y + unit * 0.75, stroke, Color::WHITE);
                canvas.draw_line(center_x - unit * 0.25, center_y + unit * 0.75, center_x + unit, center_y - unit * 0.7, stroke, Color::WHITE);
            }
            BadgeSymbol::Gear => {
                let hub_radius = inner_radius * 0.55;
                for tooth in 0..8 {
                    let angle = tooth as f32 * std::f32::consts::FRAC_PI_4;
                    let (sine, cosine) = angle.sin_cos();
                    canvas.draw_line(
                        center_x + cosine * hub_radius * 0.6,
                        center_y + sine * hub_radius * 0.6,
                        center_x + cosine * inner_radius * 0.9,
                        center_y + sine * inner_radius * 0.9,
                        stroke,
                        Color::WHITE,
                    );
                }
                canvas.fill_circle(center_x, center_y, hub_radius, Color::WHITE);
                canvas.fill_circle(center_x, center_y, hub_radius * 0.4, tint);
            }
            BadgeSymbol::Wave => {
                let unit = inner_radius * 0.55;
                canvas.draw_line(center_x - unit, center_y + unit * 0.2, center_x - unit * 0.35, center_y - unit * 0.4, stroke, Color::WHITE);
                canvas.draw_line(center_x - unit * 0.35, center_y - unit * 0.4, center_x + unit * 0.35, center_y + unit * 0.4, stroke, Color::WHITE);
                canvas.draw_line(center_x + unit * 0.35, center_y + unit * 0.4, center_x + unit, center_y - unit * 0.2, stroke, Color::WHITE);
            }
            BadgeSymbol::Glyph(glyph) => {
                self.badge_font.set_pixel_size(STATUS_BADGE_ICON_SIZE * scale);
                let glyph_width = self.badge_font.measure(glyph);
                let glyph_top = center_y - self.badge_font.line_height() / 2.0;
                canvas.draw_text(&mut self.badge_font, glyph, center_x - glyph_width / 2.0, glyph_top, Color::WHITE);
            }
        }
    }

    // MARK: - Input

    fn handle_mouse_down(&mut self) {
        let (x, y, width, height) = self.pet_rect;
        let inside_pet = self.cursor.0 >= x as f64
            && self.cursor.0 < (x + width) as f64
            && self.cursor.1 >= y as f64
            && self.cursor.1 < (y + height) as f64;
        if inside_pet {
            super::log("click -> pet");
            let phrases = self.pet.pet_phrases();
            self.model.pet_clicked(&phrases, entropy_seed(), now_epoch_secs());
            self.sync_window_size();
        }
        // Dragging anywhere (pet included) moves the window, like the Swift panel.
        if let Some(window) = &self.window {
            let _ = window.drag_window();
        }
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
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => self.handle_mouse_down(),
            WindowEvent::Moved(position) => self.remember_origin(position),
            WindowEvent::ScaleFactorChanged { .. } => self.sync_window_size(),
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
        if !event_loop.exiting() {
            event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_tick));
        }
    }
}

/// Nanosecond clock entropy for picking a random pet phrase (no rand dependency).
fn entropy_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos() as u64 ^ duration.as_secs())
        .unwrap_or(0)
}
