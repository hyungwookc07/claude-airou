//! macOS presenter: puts the software canvas on screen with real per-pixel alpha.
//!
//! The winit window's `NSView` is made layer-backed and its `CALayer.contents` is set to a
//! `CGImage` wrapping the canvas on every redraw. Together with a non-opaque NSWindow
//! (clear background, no shadow) this gives the same free-floating look as the Swift
//! overlay's `NSPanel` — no card, only the drawn pixels are visible.
//!
//! This file (plus `apply_window_behavior` below) is the one place with AppKit-specific
//! calls; a Windows/Linux presenter would replace exactly this module.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, Message};
use objc2_app_kit::{
    NSAlert, NSAlertStyle, NSApplication, NSColor, NSColorSpace, NSScreen, NSView, NSWindow,
    NSWindowCollectionBehavior,
};
use objc2_core_foundation::{CFData, CFRetained, CFType};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use objc2_core_graphics::{
    CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage, CGImageAlphaInfo,
    CGImageByteOrderInfo,
};
use objc2_quartz_core::CALayer;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

use super::draw::Canvas;
use super::placement::ScreenRect;

/// Owns the layer that displays the canvas.
pub struct LayerPresenter {
    layer: Retained<CALayer>,
}

impl LayerPresenter {
    /// Makes the window transparent (non-opaque, clear background, no shadow, follows all
    /// Spaces / full-screen apps) and attaches a content layer to its view.
    pub fn attach(window: &Window) -> Option<LayerPresenter> {
        let view = content_view(window)?;
        // SAFETY: `view` is the live NSView of a winit window and we are on the main
        // thread (winit's event loop); AppKit calls below are plain property setters.
        unsafe {
            if let Some(ns_window) = view.window() {
                apply_window_behavior(&ns_window);
            }
            view.setWantsLayer(true);
        }
        let layer = view.layer()?;
        layer.setOpaque(false);
        Some(LayerPresenter { layer })
    }

    /// Uploads `canvas` (premultiplied BGRA, see `draw.rs`) as the layer's contents.
    pub fn present(&self, canvas: &Canvas, scale_factor: f64) {
        if canvas.width == 0 || canvas.height == 0 {
            return;
        }
        let Some(image) = cg_image_from_canvas(canvas) else { return };
        self.layer.setContentsScale(scale_factor);
        // CFType is toll-free bridged to id, so the CGImage can be handed to CoreAnimation as-is.
        let image_as_cf_type: &CFType = image.as_ref();
        let image_object: &AnyObject = image_as_cf_type.as_ref();
        // SAFETY: CGImage is a valid CALayer.contents object (CFType bridged to id);
        // the layer retains it, and CoreAnimation reads it on the main thread only.
        unsafe { self.layer.setContents(Some(image_object)) };
    }
}

/// Direct access to the winit window's `NSWindow`, in AppKit screen coordinates (points,
/// bottom-left origin) — the same convention the Swift overlay stores in `config.json`.
/// Positioning goes through here instead of winit so no coordinate flip is involved.
pub struct AppKitWindow {
    ns_window: Retained<NSWindow>,
    view: Retained<NSView>,
}

impl AppKitWindow {
    pub fn from_winit(window: &Window) -> Option<AppKitWindow> {
        let view = content_view(window)?;
        let ns_window = view.window()?;
        Some(AppKitWindow { ns_window, view })
    }

    /// The window frame in AppKit screen coordinates.
    pub fn frame(&self) -> ScreenRect {
        rect_from_ns(self.ns_window.frame())
    }

    pub fn set_frame_origin(&self, x: f64, y: f64) {
        self.ns_window.setFrameOrigin(NSPoint::new(x, y));
    }

    /// `setFrame(_:display:false)`: the next display pass draws the new content into the
    /// new frame instead of flashing the old content centred in it for one frame.
    pub fn set_frame(&self, frame: ScreenRect) {
        self.ns_window.setFrame_display(
            NSRect::new(NSPoint::new(frame.x, frame.y), NSSize::new(frame.width, frame.height)),
            false,
        );
    }

    /// Pops the menu up as a context menu at the mouse location (right-click on the pet).
    pub fn show_context_menu(&self, menu: &tray_icon::menu::Menu) {
        use tray_icon::menu::ContextMenu;
        let view_pointer: *const NSView = &*self.view;
        // SAFETY: the pointer is a live NSView of this window; muda only reads it on the
        // main thread while the menu is up.
        unsafe { menu.show_context_menu_for_nsview(view_pointer.cast(), None) };
    }
}

fn rect_from_ns(rect: NSRect) -> ScreenRect {
    ScreenRect::new(rect.origin.x, rect.origin.y, rect.size.width, rect.size.height)
}

/// `NSScreen.screens.map(\.visibleFrame)` — every display's area minus menu bar and Dock.
pub fn screen_visible_frames() -> Vec<ScreenRect> {
    let Some(main_thread) = MainThreadMarker::new() else { return Vec::new() };
    NSScreen::screens(main_thread)
        .iter()
        .map(|screen| rect_from_ns(screen.visibleFrame()))
        .collect()
}

/// `(NSScreen.main ?? NSScreen.screens.first)?.visibleFrame`, with Swift's fallback rectangle.
pub fn main_screen_visible_frame() -> ScreenRect {
    let fallback = ScreenRect::new(0.0, 0.0, 1280.0, 800.0);
    let Some(main_thread) = MainThreadMarker::new() else { return fallback };
    let main_screen = NSScreen::mainScreen(main_thread).or_else(|| NSScreen::screens(main_thread).firstObject());
    main_screen.map(|screen| rect_from_ns(screen.visibleFrame())).unwrap_or(fallback)
}

/// The user's accent colour (`NSColor.controlAccentColor`) as sRGB bytes; None off the main thread.
pub fn accent_color_rgb() -> Option<(u8, u8, u8)> {
    MainThreadMarker::new()?;
    srgb_bytes(&NSColor::controlAccentColor())
}

/// `NSColor.windowBackgroundColor` resolved for the app's current appearance — the fill
/// of the Swift overlay's bubble and capsules; None off the main thread.
pub fn window_background_rgb() -> Option<(u8, u8, u8)> {
    MainThreadMarker::new()?;
    srgb_bytes(&NSColor::windowBackgroundColor())
}

fn srgb_bytes(color: &NSColor) -> Option<(u8, u8, u8)> {
    let srgb = color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
    let channel = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    Some((channel(srgb.redComponent()), channel(srgb.greenComponent()), channel(srgb.blueComponent())))
}

/// Modal alert like the Swift menu actions show (activates the app first so it is visible).
pub fn show_alert(title: &str, body: &str, is_warning: bool) {
    let Some(main_thread) = MainThreadMarker::new() else { return };
    let alert = NSAlert::new(main_thread);
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(body));
    if is_warning {
        alert.setAlertStyle(NSAlertStyle::Warning);
    }
    #[allow(deprecated)]
    NSApplication::sharedApplication(main_thread).activateIgnoringOtherApps(true);
    let _ = alert.runModal();
}

/// Whether the app currently renders in the dark appearance (drives the capsule palette).
/// Off the main thread (never the case for the overlay) this defaults to dark.
pub fn is_dark_appearance() -> bool {
    let Some(main_thread) = MainThreadMarker::new() else { return true };
    let appearance_name = NSApplication::sharedApplication(main_thread).effectiveAppearance().name();
    appearance_name.to_string().contains("Dark")
}

fn content_view(window: &Window) -> Option<Retained<NSView>> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else { return None };
    // SAFETY: winit guarantees `ns_view` points at a valid NSView for the window's lifetime;
    // retaining it hands us an owned reference that outlives this function.
    let view: &NSView = unsafe { &*appkit.ns_view.as_ptr().cast::<NSView>() };
    Some(view.retain())
}

/// NSWindow tweaks that winit does not expose: transparent, shadowless, and
/// collectionBehavior = canJoinAllSpaces | fullScreenAuxiliary | stationary so the pet
/// follows the user across Spaces and stays over full-screen apps.
///
/// # Safety
/// Must be called on the main thread with a live NSWindow.
unsafe fn apply_window_behavior(ns_window: &NSWindow) {
    ns_window.setOpaque(false);
    ns_window.setBackgroundColor(Some(&NSColor::clearColor()));
    ns_window.setHasShadow(false);
    ns_window.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Stationary,
    );
}

/// Wraps the canvas pixels in a CGImage. The canvas is `0xAARRGGBB` premultiplied in
/// native (little-endian) order, i.e. bytes B,G,R,A = `ByteOrder32Little | PremultipliedFirst`.
fn cg_image_from_canvas(canvas: &Canvas) -> Option<CFRetained<CGImage>> {
    const BITS_PER_COMPONENT: usize = 8;
    const BITS_PER_PIXEL: usize = 32;
    let bytes: &[u8] = bytemuck_cast_pixels(&canvas.pixels);
    let data = CFData::from_bytes(bytes);
    let provider = CGDataProvider::with_cf_data(Some(&data))?;
    let color_space = CGColorSpace::new_device_rgb()?;
    let bitmap_info =
        CGBitmapInfo(CGImageByteOrderInfo::Order32Little.0 | CGImageAlphaInfo::PremultipliedFirst.0);
    // SAFETY: `decode` is null (allowed); all sizes describe `bytes` exactly:
    // width × height pixels of 4 bytes, `bytes_per_row = width × 4`.
    unsafe {
        CGImage::new(
            canvas.width as usize,
            canvas.height as usize,
            BITS_PER_COMPONENT,
            BITS_PER_PIXEL,
            canvas.width as usize * 4,
            Some(&color_space),
            bitmap_info,
            Some(&provider),
            std::ptr::null(),
            false,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
    }
}

/// Views the pixel buffer as bytes without copying (u32 → 4 × u8, native endianness).
fn bytemuck_cast_pixels(pixels: &[u32]) -> &[u8] {
    // SAFETY: u32 has no padding and any byte pattern is a valid u8; the slice covers
    // exactly `pixels.len() * 4` bytes and lives as long as `pixels`.
    unsafe { std::slice::from_raw_parts(pixels.as_ptr().cast::<u8>(), pixels.len() * 4) }
}
