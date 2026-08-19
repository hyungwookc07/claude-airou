//! Panel placement math in AppKit screen coordinates (points, origin bottom-left of the
//! main display, y up) — the convention the Swift overlay stores in `config.json`
//! (`windowOriginX` / `windowOriginY`), so both overlays put the pet in the same place.
//! Port of the geometry helpers in `OverlayPanel.swift`; pure so it is unit-tested.

/// A rectangle in AppKit screen coordinates (bottom-left origin, y up).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl ScreenRect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> ScreenRect {
        ScreenRect { x, y, width, height }
    }

    pub fn min_x(&self) -> f64 {
        self.x
    }

    pub fn max_x(&self) -> f64 {
        self.x + self.width
    }

    pub fn min_y(&self) -> f64 {
        self.y
    }

    pub fn max_y(&self) -> f64 {
        self.y + self.height
    }

    /// `NSRect.insetBy(dx:dy:)`.
    pub fn inset_by(&self, dx: f64, dy: f64) -> ScreenRect {
        ScreenRect::new(self.x + dx, self.y + dy, self.width - 2.0 * dx, self.height - 2.0 * dy)
    }

    /// `NSRect.intersects` (empty rectangles never intersect).
    pub fn intersects(&self, other: &ScreenRect) -> bool {
        if self.width <= 0.0 || self.height <= 0.0 || other.width <= 0.0 || other.height <= 0.0 {
            return false;
        }
        self.min_x() < other.max_x()
            && other.min_x() < self.max_x()
            && self.min_y() < other.max_y()
            && other.min_y() < self.max_y()
    }
}

/// Margin from the screen corner for the default position (Swift: 24 pt).
pub const DEFAULT_ORIGIN_MARGIN: f64 = 24.0;

/// Bottom-right corner of the main screen's visible area, with a margin
/// (`OverlayPanel.defaultOrigin(for:)`; `visible` = `NSScreen.main.visibleFrame`).
pub fn default_origin(panel_width: f64, visible: ScreenRect) -> (f64, f64) {
    (visible.max_x() - panel_width - DEFAULT_ORIGIN_MARGIN, visible.min_y() + DEFAULT_ORIGIN_MARGIN)
}

/// Require a meaningful chunk of the panel to intersect a screen so it stays reachable
/// (`OverlayPanel.isRectVisible`).
pub fn is_rect_visible(rect: ScreenRect, screens: &[ScreenRect]) -> bool {
    let probe = rect.inset_by(rect.width * 0.25, rect.height * 0.25);
    screens.iter().any(|screen| screen.intersects(&probe))
}

/// Where the panel goes on launch: the saved origin if it keeps the panel on some screen,
/// otherwise the default corner (`OverlayPanel.place(at:)`).
pub fn placement_origin(
    saved_origin: Option<(f64, f64)>,
    panel_width: f64,
    panel_height: f64,
    screens: &[ScreenRect],
    main_visible: ScreenRect,
) -> (f64, f64) {
    if let Some((x, y)) = saved_origin {
        if is_rect_visible(ScreenRect::new(x, y, panel_width, panel_height), screens) {
            return (x, y);
        }
    }
    default_origin(panel_width, main_visible)
}

/// New frame for a resize that keeps the content point `content_x` (points from the left)
/// at the same screen x and the bottom edge where it was
/// (`OverlayPanel.resize(to:keepingContentX:atScreenX:)`).
pub fn frame_keeping_content_x(
    current: ScreenRect,
    new_width: f64,
    new_height: f64,
    content_x: f64,
    screen_x: f64,
) -> ScreenRect {
    ScreenRect::new((screen_x - content_x).round(), current.min_y(), new_width, new_height)
}

/// Squared distance between two rectangles (0 when they touch or overlap).
fn distance_squared(rect: &ScreenRect, other: &ScreenRect) -> f64 {
    let dx = (other.min_x() - rect.max_x()).max(rect.min_x() - other.max_x()).max(0.0);
    let dy = (other.min_y() - rect.max_y()).max(rect.min_y() - other.max_y()).max(0.0);
    dx * dx + dy * dy
}

/// Slides the frame back inside the nearest screen's visible area if it grew or was
/// restored partly off-screen (`OverlayPanel.nudgeOntoScreen`). Returns the origin to
/// use (unchanged when already inside, or when there is no screen at all).
pub fn nudged_origin(frame: ScreenRect, screens: &[ScreenRect]) -> (f64, f64) {
    let nearest = screens.iter().min_by(|left, right| {
        distance_squared(&frame, left)
            .partial_cmp(&distance_squared(&frame, right))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let Some(visible) = nearest else { return (frame.x, frame.y) };
    let x = frame
        .x
        .max(visible.min_x())
        .min(visible.min_x().max(visible.max_x() - frame.width));
    let y = frame
        .y
        .max(visible.min_y())
        .min(visible.min_y().max(visible.max_y() - frame.height));
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Visible frame: menu bar (37) removed from the top, dock (70) from the bottom.
    const MAIN_VISIBLE: ScreenRect = ScreenRect { x: 0.0, y: 70.0, width: 1728.0, height: 1010.0 };
    /// A second display to the left of the main one, slightly higher.
    const LEFT: ScreenRect = ScreenRect { x: -1920.0, y: 200.0, width: 1920.0, height: 1080.0 };

    #[test]
    fn rect_helpers() {
        let rect = ScreenRect::new(10.0, 20.0, 100.0, 50.0);
        assert_eq!((rect.min_x(), rect.max_x(), rect.min_y(), rect.max_y()), (10.0, 110.0, 20.0, 70.0));
        assert_eq!(rect.inset_by(10.0, 5.0), ScreenRect::new(20.0, 25.0, 80.0, 40.0));
        assert!(rect.intersects(&ScreenRect::new(100.0, 60.0, 50.0, 50.0)));
        assert!(!rect.intersects(&ScreenRect::new(110.0, 20.0, 50.0, 50.0)), "touching edges do not intersect");
        assert!(!rect.intersects(&ScreenRect::new(0.0, 0.0, 0.0, 0.0)));
    }

    #[test]
    fn default_origin_is_bottom_right_of_visible_area_with_margin() {
        let (x, y) = default_origin(220.0, MAIN_VISIBLE);
        assert_eq!(x, 1728.0 - 220.0 - 24.0);
        assert_eq!(y, 70.0 + 24.0);
    }

    #[test]
    fn saved_origin_is_kept_only_when_a_quarter_inset_probe_hits_a_screen() {
        let screens = [MAIN_VISIBLE, LEFT];
        // Comfortably on the main screen.
        assert_eq!(placement_origin(Some((1294.0, 1014.0)), 220.0, 240.0, &screens, MAIN_VISIBLE), (1294.0, 1014.0));
        // On the left display (negative x is fine).
        assert_eq!(placement_origin(Some((-800.0, 500.0)), 220.0, 240.0, &screens, MAIN_VISIBLE), (-800.0, 500.0));
        // Mostly off the bottom of the main screen: probe (inset 25 %) misses everything.
        let fallback = default_origin(220.0, MAIN_VISIBLE);
        assert_eq!(placement_origin(Some((500.0, -230.0)), 220.0, 240.0, &screens, MAIN_VISIBLE), fallback);
        // Far away (an unplugged monitor).
        assert_eq!(placement_origin(Some((5000.0, 100.0)), 220.0, 240.0, &screens, MAIN_VISIBLE), fallback);
        // Nothing saved.
        assert_eq!(placement_origin(None, 220.0, 240.0, &screens, MAIN_VISIBLE), fallback);
        // A rect hanging half off the right edge still counts as visible (probe overlaps).
        assert!(is_rect_visible(ScreenRect::new(1728.0 - 120.0, 500.0, 220.0, 240.0), &screens));
        assert!(!is_rect_visible(ScreenRect::new(1728.0 - 40.0, 500.0, 220.0, 240.0), &screens));
    }

    #[test]
    fn resize_keeps_content_x_on_screen_and_bottom_edge() {
        let current = ScreenRect::new(1000.0, 300.0, 220.0, 240.0);
        // Primary centre was at content x 110 -> screen 1110; new layout puts it at 176 in a 352-wide panel.
        let resized = frame_keeping_content_x(current, 352.0, 240.0, 176.0, 1110.0);
        assert_eq!(resized, ScreenRect::new(934.0, 300.0, 352.0, 240.0));
        assert_eq!(resized.min_x() + 176.0, 1110.0, "primary pet did not move");
        // Height changes keep the bottom edge (Swift keeps frame.minY).
        let taller = frame_keeping_content_x(current, 220.0, 256.0, 110.0, 1110.0);
        assert_eq!(taller.min_y(), 300.0);
        assert_eq!(taller.height, 256.0);
        // Fractional anchors are rounded to whole points.
        assert_eq!(frame_keeping_content_x(current, 220.0, 240.0, 110.4, 1110.0).x, 1000.0);
    }

    #[test]
    fn nudge_pulls_the_frame_back_inside_the_nearest_screen() {
        let screens = [MAIN_VISIBLE, LEFT];
        // Already inside: unchanged.
        assert_eq!(nudged_origin(ScreenRect::new(100.0, 100.0, 220.0, 240.0), &screens), (100.0, 100.0));
        // Grew past the right edge: slides left.
        assert_eq!(nudged_origin(ScreenRect::new(1600.0, 100.0, 352.0, 240.0), &screens), (1728.0 - 352.0, 100.0));
        // Below the dock: slides up onto the visible area.
        assert_eq!(nudged_origin(ScreenRect::new(100.0, 10.0, 220.0, 240.0), &screens), (100.0, 70.0));
        // Above the menu bar: slides down.
        assert_eq!(nudged_origin(ScreenRect::new(100.0, 1000.0, 220.0, 240.0), &screens), (100.0, 1080.0 - 240.0));
        // On the left display, near its top: nearest screen is LEFT, so it clamps to LEFT.
        assert_eq!(nudged_origin(ScreenRect::new(-500.0, 1200.0, 220.0, 240.0), &screens), (-500.0, 1280.0 - 240.0));
        // Wider than the screen: pinned to the screen's left edge.
        assert_eq!(nudged_origin(ScreenRect::new(-100.0, 100.0, 3000.0, 240.0), &[MAIN_VISIBLE]), (0.0, 100.0));
        // No screens: nothing to do.
        assert_eq!(nudged_origin(ScreenRect::new(-100.0, -100.0, 220.0, 240.0), &[]), (-100.0, -100.0));
    }
}
