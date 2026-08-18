//! The overlay window and event loop (winit 0.30 + softbuffer 0.4 + tray-icon).
//!
//! PROMINENT NOTE — transparency fallback: softbuffer's buffer format is `0RGB`
//! (no alpha), so true per-pixel window transparency is not achievable with
//! softbuffer alone on macOS. The overlay deliberately draws an opaque dark
//! rounded-card look instead of a transparent window (see `draw.rs` header). This is
//! the compile-clean pragmatic path the spec allows; swap-in of a transparent
//! renderer is a future improvement.
//!
//! All platform-specific tweaks live here so a future `overlay/windows.rs` sibling
//! only has to replace this file's corners:
//! - activation policy Accessory (never steals focus),
//! - NSWindow.collectionBehavior canJoinAllSpaces | fullScreenAuxiliary | stationary
//!   (the single `unsafe` block, in `apply_macos_window_behavior`).

use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS, WindowAttributesExtMacOS};
use winit::window::{Window, WindowId, WindowLevel};

use super::{draw, lock, logic::OverlayModel, tray};
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

// Card palette (opaque fallback, see module note).
const COLOR_BACKDROP: u32 = 0; // black behind the rounded corners
const COLOR_CARD: u32 = 0x0020_222A;
const COLOR_BUBBLE: u32 = 0x0034_3742;
const COLOR_TEXT: u32 = 0x00EB_ECF0;
const COLOR_LABEL: u32 = 0x00AA_AFB9;
const COLOR_GAUGE_TRACK: u32 = 0x003C_404A;
const COLOR_GREEN: u32 = 0x0034_C759;
const COLOR_ORANGE: u32 = 0x00FF_9F0A;
const COLOR_RED: u32 = 0x00FF_453A;
const COLOR_WHITE: u32 = 0x00FF_FFFF;

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
        surface: None,
        tray_icon: None,
        menu_model: None,
        next_tick: Instant::now() + TICK,
        tick_count: 0,
        cursor: (0.0, 0.0),
        pet_rect: (0, 0, 0, 0),
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
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    tray_icon: Option<tray_icon::TrayIcon>,
    menu_model: Option<tray::MenuModel>,
    next_tick: Instant,
    tick_count: u64,
    cursor: (f64, f64),
    /// Pet sprite hit-box in physical pixels: (x, y, w, h), updated on every redraw.
    pet_rect: (i32, i32, i32, i32),
    config_dirty_at: Option<Instant>,
    lock_path: std::path::PathBuf,
    pid: u32,
    exit_code: i32,
}

impl App {
    // MARK: - Layout (physical pixels)

    fn scale_factor(&self) -> f64 {
        self.window.as_ref().map(|window| window.scale_factor()).unwrap_or(1.0)
    }

    fn px(&self, logical: f64) -> i32 {
        ((logical * self.scale_factor()).round() as i32).max(0)
    }

    fn sprite_scale(&self) -> u32 {
        ((self.config.pixel_scale * self.scale_factor()).round() as i64).max(1) as u32
    }

    fn text_scale(&self) -> i32 {
        ((self.scale_factor() * 1.5).round() as i32).max(1)
    }

    fn label_scale(&self) -> i32 {
        (self.scale_factor().round() as i32).max(1)
    }

    fn grid_size(&self) -> (u32, u32) {
        let (w, h) = self.pet.grid_size();
        (w.max(1) as u32, h.max(1) as u32)
    }

    fn shows_gauge(&self) -> bool {
        self.config.gauge_metric != GaugeMetric::Off
    }

    /// Desired window size in physical pixels for the current pet / scale / gauge.
    /// The bubble area is always reserved (two text lines) so the window does not
    /// resize every time a message appears or disappears.
    fn desired_size(&self) -> PhysicalSize<u32> {
        let (grid_w, grid_h) = self.grid_size();
        let sprite = self.sprite_scale();
        let pad = self.px(8.0);
        let pet_w = (grid_w * sprite) as i32;
        let pet_h = (grid_h * sprite) as i32;
        let bubble_h = self.bubble_height();
        let gauge_h = if self.shows_gauge() { self.px(4.0) + self.px(3.0) } else { 0 };
        let label_h = 8 * self.label_scale() + self.px(2.0);
        let width = pet_w.max(self.px(190.0)) + 2 * pad;
        let height = pad + bubble_h + self.px(4.0) + pet_h + gauge_h + label_h + pad;
        PhysicalSize::new(width.max(1) as u32, height.max(1) as u32)
    }

    fn bubble_height(&self) -> i32 {
        2 * 8 * self.text_scale() + 2 * self.px(4.0)
    }

    fn sync_window_size(&self) {
        let Some(window) = &self.window else { return };
        let desired = self.desired_size();
        if window.inner_size() != desired {
            let _ = window.request_inner_size(desired);
        }
        window.request_redraw();
    }

    // MARK: - Startup

    fn create_window(&mut self, event_loop: &ActiveEventLoop) {
        let saved_origin = match (self.config.window_origin_x, self.config.window_origin_y) {
            (Some(x), Some(y)) => Some(LogicalPosition::new(x, y)),
            _ => None,
        };
        let mut attributes = Window::default_attributes()
            .with_title("Claude Airou")
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
        apply_macos_window_behavior(&window);
        if self.config.is_click_through {
            let _ = window.set_cursor_hittest(false);
        }

        let context = match softbuffer::Context::new(window.clone()) {
            Ok(context) => context,
            Err(error) => {
                crate::logging::eprint_line(&format!(
                    "claude-airou: could not create the drawing context: {error}"
                ));
                self.exit_code = 1;
                event_loop.exit();
                return;
            }
        };
        match softbuffer::Surface::new(&context, window.clone()) {
            Ok(surface) => self.surface = Some(surface),
            Err(error) => {
                crate::logging::eprint_line(&format!(
                    "claude-airou: could not create the drawing surface: {error}"
                ));
                self.exit_code = 1;
                event_loop.exit();
                return;
            }
        }
        self.window = Some(window);
        self.sync_window_size();
        if saved_origin.is_none() {
            self.reset_position();
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
        let margin = self.px(24.0);
        let x = monitor_position.x + monitor_size.width as i32 - window_size.width as i32 - margin;
        let y = monitor_position.y + monitor_size.height as i32 - window_size.height as i32 - margin;
        window.set_outer_position(PhysicalPosition::new(x, y));
        self.remember_origin(PhysicalPosition::new(x, y));
    }

    fn remember_origin(&mut self, position: PhysicalPosition<i32>) {
        let sf = self.scale_factor();
        self.config.window_origin_x = Some(position.x as f64 / sf);
        self.config.window_origin_y = Some(position.y as f64 / sf);
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

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
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
        }
        if let Some(dirty_at) = self.config_dirty_at {
            if dirty_at.elapsed() >= CONFIG_SAVE_DEBOUNCE {
                self.config_dirty_at = None;
                self.config.save();
            }
        }

        self.sync_tray_menu();
        self.request_redraw();
    }

    // MARK: - Drawing

    fn redraw(&mut self) {
        let Some(window) = self.window.clone() else { return };
        let size = window.inner_size();
        let (Some(width), Some(height)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return;
        };

        let mut canvas = draw::Canvas::new(size.width, size.height);
        self.paint(&mut canvas);

        let Some(surface) = self.surface.as_mut() else { return };
        if surface.resize(width, height).is_err() {
            return;
        }
        let Ok(mut buffer) = surface.buffer_mut() else { return };
        if buffer.len() == canvas.pixels.len() {
            buffer.copy_from_slice(&canvas.pixels);
        }
        let _ = buffer.present();
    }

    fn paint(&mut self, canvas: &mut draw::Canvas) {
        let width = canvas.width as i32;
        let height = canvas.height as i32;
        let pad = self.px(8.0);
        let text_scale = self.text_scale();
        let label_scale = self.label_scale();

        // Opaque rounded card (transparency fallback — see module note).
        canvas.fill(COLOR_BACKDROP);
        canvas.fill_rounded_rect(0, 0, width, height, self.px(10.0), COLOR_CARD);

        // Speech bubble (area is always reserved; drawn only when visible).
        let bubble_h = self.bubble_height();
        if self.model.is_speech_bubble_visible(self.config.is_speech_bubble_hidden) {
            let bubble_x = pad;
            let bubble_w = width - 2 * pad;
            canvas.fill_rounded_rect(bubble_x, pad, bubble_w, bubble_h, self.px(6.0), COLOR_BUBBLE);
            let inner_pad = self.px(4.0);
            let max_chars = ((bubble_w - 2 * inner_pad) / (8 * text_scale)).max(4) as usize;
            let lines = draw::wrap_text(self.model.speech_text(), max_chars, 2);
            for (index, line) in lines.iter().enumerate() {
                canvas.draw_text(
                    line,
                    bubble_x + inner_pad,
                    pad + inner_pad + index as i32 * 8 * text_scale,
                    text_scale,
                    COLOR_TEXT,
                );
            }
        }

        // Pet sprite, centred horizontally.
        let sprite_scale = self.sprite_scale();
        let (grid_w, grid_h) = self.grid_size();
        let pet_w = (grid_w * sprite_scale) as i32;
        let pet_h = (grid_h * sprite_scale) as i32;
        let pet_x = (width - pet_w) / 2;
        let pet_y = pad + bubble_h + self.px(4.0);
        self.pet_rect = (pet_x, pet_y, pet_w, pet_h);

        let frames = self.pet.frames_for(self.model.display_state);
        if !frames.is_empty() {
            let frame = &frames[self.model.frame_index % frames.len()];
            if let Ok((rgba, image_w, image_h)) =
                crate::render::frame_rgba(frame, &self.palette, sprite_scale, None)
            {
                canvas.blit_rgba(&rgba, image_w, image_h, pet_x, pet_y);
            }
        }

        self.paint_badge(canvas, pet_x + pet_w, pet_y);

        // Battery gauge under the pet.
        let mut label_y = pet_y + pet_h + self.px(2.0);
        if self.shows_gauge() {
            let gauge_y = pet_y + pet_h + self.px(3.0);
            let gauge_h = self.px(4.0).max(2);
            canvas.fill_rounded_rect(pet_x, gauge_y, pet_w, gauge_h, gauge_h / 2, COLOR_GAUGE_TRACK);
            if let Some(value) = self.model.gauge_value(self.config.gauge_metric) {
                let clamped = value.clamp(0.0, 100.0);
                let fill_w = ((pet_w as f64) * clamped / 100.0).round() as i32;
                let color = if clamped >= 50.0 {
                    COLOR_GREEN
                } else if clamped >= 20.0 {
                    COLOR_ORANGE
                } else {
                    COLOR_RED
                };
                canvas.fill_rounded_rect(pet_x, gauge_y, fill_w, gauge_h, gauge_h / 2, color);
            }
            label_y = gauge_y + gauge_h + self.px(2.0);
        }

        // Session label ("project", "project +N", "no session"), centred.
        let label = self.model.collapsed_label();
        let label_w = draw::text_width(&label, label_scale);
        canvas.draw_text(&label, (width - label_w) / 2, label_y, label_scale, COLOR_LABEL);
    }

    /// Status badge at the pet's top-right corner: red clock (waiting approval),
    /// orange ? (needs input), green check (done), red ! (error). Busy states rely on
    /// the sprite's own frames, like the Swift PetView.
    fn paint_badge(&self, canvas: &mut draw::Canvas, corner_x: i32, corner_y: i32) {
        let radius = self.px(7.0).max(4);
        let cx = corner_x - radius / 2;
        let cy = corner_y + radius / 2;
        let line = (radius / 3).max(1);
        match self.model.display_state {
            PetState::WaitingApproval => {
                canvas.fill_circle(cx, cy, radius, COLOR_RED);
                // Clock hands: minute up, hour to the right.
                canvas.draw_line(cx, cy, cx, cy - (radius * 6) / 10, line, COLOR_WHITE);
                canvas.draw_line(cx, cy, cx + (radius * 4) / 10, cy, line, COLOR_WHITE);
            }
            PetState::NeedsInput => {
                canvas.fill_circle(cx, cy, radius, COLOR_ORANGE);
                let scale = ((radius * 2) / 8).max(1);
                canvas.draw_text("?", cx - 4 * scale, cy - 4 * scale, scale, COLOR_WHITE);
            }
            PetState::Done => {
                canvas.fill_circle(cx, cy, radius, COLOR_GREEN);
                let dx = radius / 2;
                canvas.draw_line(cx - dx, cy, cx - dx / 4, cy + dx, line, COLOR_WHITE);
                canvas.draw_line(cx - dx / 4, cy + dx, cx + dx, cy - dx / 2, line, COLOR_WHITE);
            }
            PetState::Error => {
                canvas.fill_circle(cx, cy, radius, COLOR_RED);
                let scale = ((radius * 2) / 8).max(1);
                canvas.draw_text("!", cx - 4 * scale, cy - 4 * scale, scale, COLOR_WHITE);
            }
            _ => {}
        }
    }

    // MARK: - Input

    fn handle_mouse_down(&mut self) {
        let (x, y, w, h) = self.pet_rect;
        let inside_pet = self.cursor.0 >= x as f64
            && self.cursor.0 < (x + w) as f64
            && self.cursor.1 >= y as f64
            && self.cursor.1 < (y + h) as f64;
        if inside_pet {
            super::log("click -> pet");
            let phrases = self.pet.pet_phrases();
            self.model.pet_clicked(&phrases, entropy_seed(), now_epoch_secs());
            self.request_redraw();
        }
        // Dragging anywhere (pet included) moves the window, like the Swift panel.
        if let Some(window) = &self.window {
            let _ = window.drag_window();
        }
    }
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

/// The one `unsafe` corner: reach the NSWindow behind the winit window and set
/// collectionBehavior = canJoinAllSpaces | fullScreenAuxiliary | stationary so the pet
/// follows the user across Spaces and stays over full-screen apps.
fn apply_macos_window_behavior(window: &Window) {
    use objc2_app_kit::{NSView, NSWindowCollectionBehavior};
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let Ok(handle) = window.window_handle() else { return };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else { return };
    let behavior = NSWindowCollectionBehavior::CanJoinAllSpaces
        | NSWindowCollectionBehavior::FullScreenAuxiliary
        | NSWindowCollectionBehavior::Stationary;
    // SAFETY: winit guarantees ns_view is a valid NSView pointer for the lifetime of
    // the window, and we are on the main thread (winit event loop on macOS).
    unsafe {
        let view: &NSView = &*appkit.ns_view.as_ptr().cast();
        if let Some(ns_window) = view.window() {
            ns_window.setCollectionBehavior(behavior);
        }
    }
}
