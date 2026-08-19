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
use objc2_app_kit::{NSApplication, NSColor, NSView, NSWindow, NSWindowCollectionBehavior};
use objc2_core_foundation::{CFData, CFRetained, CFType};
use objc2_core_graphics::{
    CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage, CGImageAlphaInfo,
    CGImageByteOrderInfo,
};
use objc2_quartz_core::CALayer;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

use super::draw::Canvas;

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
