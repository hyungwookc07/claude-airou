//! Software compositor for the overlay: an RGBA canvas with real alpha, so the pet,
//! bubble, label and gauge float over the desktop like the Swift overlay.
//!
//! Pixel format: `Vec<u32>` of **premultiplied** ARGB, packed `0xAARRGGBB` in the CPU's
//! native endianness. On little-endian machines that is the byte order B, G, R, A —
//! exactly Core Graphics' `kCGBitmapByteOrder32Little | kCGImageAlphaPremultipliedFirst`
//! ("BGRA"), so `present_macos.rs` can hand the buffer to a `CGImage` without any copy or
//! swizzle. Compositing is source-over in premultiplied space
//! (`out = src + dst * (1 - src_alpha)`), which is exact for transparent destinations.
//! `Color` values in the public API are *straight* alpha; they are premultiplied on the way in.
//!
//! Anti-aliasing: rounded rectangles, capsules and circles are rasterised from a signed
//! distance field (coverage = clamp(0.5 - distance)), so the translucent capsules get
//! smooth edges like SwiftUI's shapes.

use super::text::TextRasterizer;

/// A straight-alpha sRGB color (what callers write); the canvas premultiplies internally.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

impl Color {
    #[cfg(test)]
    pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0);
    pub const WHITE: Color = Color::rgb(255, 255, 255);

    pub const fn rgb(red: u8, green: u8, blue: u8) -> Color {
        Color { red, green, blue, alpha: 255 }
    }

    pub const fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> Color {
        Color { red, green, blue, alpha }
    }

    /// Same color with its alpha scaled by `opacity` (0..=1).
    pub fn with_opacity(self, opacity: f32) -> Color {
        let alpha = (self.alpha as f32 * opacity.clamp(0.0, 1.0)).round() as u8;
        Color { alpha, ..self }
    }
}

/// Packs a straight-alpha color into a premultiplied `0xAARRGGBB` pixel.
pub fn premultiply(color: Color) -> u32 {
    let alpha = color.alpha as u32;
    let red = (color.red as u32 * alpha + 127) / 255;
    let green = (color.green as u32 * alpha + 127) / 255;
    let blue = (color.blue as u32 * alpha + 127) / 255;
    (alpha << 24) | (red << 16) | (green << 8) | blue
}

/// Unpacks a premultiplied pixel back to a straight-alpha color (0 alpha → transparent black).
#[cfg(test)]
pub fn unpremultiply(pixel: u32) -> Color {
    let alpha = pixel >> 24;
    if alpha == 0 {
        return Color::TRANSPARENT;
    }
    let channel = |value: u32| ((value * 255 + alpha / 2) / alpha).min(255) as u8;
    Color::rgba(
        channel((pixel >> 16) & 0xFF),
        channel((pixel >> 8) & 0xFF),
        channel(pixel & 0xFF),
        alpha as u8,
    )
}

/// Source-over of two premultiplied pixels: `source + destination * (1 - source_alpha)`.
pub fn blend_premultiplied(destination: u32, source: u32) -> u32 {
    let source_alpha = source >> 24;
    match source_alpha {
        0 => destination,
        255 => source,
        _ => {
            let inverse = 255 - source_alpha;
            let mix = |shift: u32| {
                let source_channel = (source >> shift) & 0xFF;
                let destination_channel = (destination >> shift) & 0xFF;
                let mixed = source_channel + (destination_channel * inverse + 127) / 255;
                mixed.min(255) << shift
            };
            mix(24) | mix(16) | mix(8) | mix(0)
        }
    }
}

/// Scales a premultiplied pixel by `coverage` (0..=1) — used for anti-aliased edges.
fn scale_premultiplied(pixel: u32, coverage: f32) -> u32 {
    if coverage >= 1.0 {
        return pixel;
    }
    if coverage <= 0.0 {
        return 0;
    }
    let scale = |shift: u32| ((((pixel >> shift) & 0xFF) as f32 * coverage).round() as u32) << shift;
    scale(24) | scale(16) | scale(8) | scale(0)
}

pub struct Canvas {
    pub width: u32,
    pub height: u32,
    /// Premultiplied `0xAARRGGBB`, row-major, top-down.
    pub pixels: Vec<u32>,
}

impl Canvas {
    /// A fully transparent canvas.
    pub fn new(width: u32, height: u32) -> Canvas {
        Canvas {
            width,
            height,
            pixels: vec![0u32; width as usize * height as usize],
        }
    }

    #[cfg(test)]
    pub fn pixel_at(&self, x: i32, y: i32) -> u32 {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return 0;
        }
        self.pixels[y as usize * self.width as usize + x as usize]
    }

    /// The straight-alpha color at (x, y); transparent outside the canvas.
    #[cfg(test)]
    pub fn color_at(&self, x: i32, y: i32) -> Color {
        unpremultiply(self.pixel_at(x, y))
    }

    /// Composites a premultiplied pixel over (x, y); no-op outside the canvas.
    fn blend_pixel(&mut self, x: i32, y: i32, premultiplied: u32) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let index = y as usize * self.width as usize + x as usize;
        self.pixels[index] = blend_premultiplied(self.pixels[index], premultiplied);
    }

    /// Clipped, alpha-blended rectangle fill.
    #[cfg(test)]
    pub fn fill_rect(&mut self, x: i32, y: i32, width: i32, height: i32, color: Color) {
        let premultiplied = premultiply(color);
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + width).min(self.width as i32);
        let y1 = (y + height).min(self.height as i32);
        for row in y0..y1 {
            for column in x0..x1 {
                self.blend_pixel(column, row, premultiplied);
            }
        }
    }

    /// Anti-aliased rounded rectangle (radius clamped to half the short side; a radius of
    /// half the height gives a capsule). `x`, `y`, `width`, `height` are pixel-edge
    /// coordinates, so a 1×1 rectangle covers exactly one pixel.
    pub fn fill_rounded_rect(&mut self, x: f32, y: f32, width: f32, height: f32, radius: f32, color: Color) {
        self.paint_rounded_rect(x, y, width, height, radius, color, None);
    }

    /// Anti-aliased rounded-rectangle outline of `line_width` pixels, drawn inside the bounds
    /// (like SwiftUI's `strokeBorder`).
    pub fn stroke_rounded_rect(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        radius: f32,
        line_width: f32,
        color: Color,
    ) {
        self.paint_rounded_rect(x, y, width, height, radius, color, Some(line_width));
    }

    /// Anti-aliased filled circle.
    pub fn fill_circle(&mut self, center_x: f32, center_y: f32, radius: f32, color: Color) {
        self.fill_rounded_rect(center_x - radius, center_y - radius, 2.0 * radius, 2.0 * radius, radius, color);
    }

    /// Anti-aliased circle outline (`line_width` inside the radius).
    pub fn stroke_circle(&mut self, center_x: f32, center_y: f32, radius: f32, line_width: f32, color: Color) {
        self.stroke_rounded_rect(
            center_x - radius,
            center_y - radius,
            2.0 * radius,
            2.0 * radius,
            radius,
            line_width,
            color,
        );
    }

    fn paint_rounded_rect(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        radius: f32,
        color: Color,
        stroke_width: Option<f32>,
    ) {
        if width <= 0.0 || height <= 0.0 || color.alpha == 0 {
            return;
        }
        let radius = radius.max(0.0).min(width / 2.0).min(height / 2.0);
        let premultiplied = premultiply(color);
        let half_width = width / 2.0;
        let half_height = height / 2.0;
        let center_x = x + half_width;
        let center_y = y + half_height;
        let x0 = (x.floor() as i32 - 1).max(0);
        let y0 = (y.floor() as i32 - 1).max(0);
        let x1 = ((x + width).ceil() as i32 + 1).min(self.width as i32);
        let y1 = ((y + height).ceil() as i32 + 1).min(self.height as i32);
        for row in y0..y1 {
            let sample_y = row as f32 + 0.5;
            for column in x0..x1 {
                let sample_x = column as f32 + 0.5;
                // Signed distance to the rounded rectangle's edge (negative inside).
                let distance = rounded_rect_signed_distance(
                    sample_x - center_x,
                    sample_y - center_y,
                    half_width,
                    half_height,
                    radius,
                );
                let outer_coverage = (0.5 - distance).clamp(0.0, 1.0);
                let coverage = match stroke_width {
                    None => outer_coverage,
                    Some(line_width) => {
                        // Stroke = outer shape minus the same shape inset by line_width.
                        let inner_coverage = (0.5 - (distance + line_width)).clamp(0.0, 1.0);
                        (outer_coverage - inner_coverage).clamp(0.0, 1.0)
                    }
                };
                if coverage > 0.0 {
                    self.blend_pixel(column, row, scale_premultiplied(premultiplied, coverage));
                }
            }
        }
    }

    /// Anti-aliased line of `thickness` pixels between two points (round caps).
    pub fn draw_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, thickness: f32, color: Color) {
        let premultiplied = premultiply(color);
        let half = thickness / 2.0;
        let min_x = (x0.min(x1) - half - 1.0).floor().max(0.0) as i32;
        let min_y = (y0.min(y1) - half - 1.0).floor().max(0.0) as i32;
        let max_x = ((x0.max(x1) + half + 1.0).ceil() as i32).min(self.width as i32);
        let max_y = ((y0.max(y1) + half + 1.0).ceil() as i32).min(self.height as i32);
        let delta_x = x1 - x0;
        let delta_y = y1 - y0;
        let length_squared = delta_x * delta_x + delta_y * delta_y;
        for row in min_y..max_y {
            let sample_y = row as f32 + 0.5;
            for column in min_x..max_x {
                let sample_x = column as f32 + 0.5;
                let projection = if length_squared <= f32::EPSILON {
                    0.0
                } else {
                    (((sample_x - x0) * delta_x + (sample_y - y0) * delta_y) / length_squared).clamp(0.0, 1.0)
                };
                let nearest_x = x0 + projection * delta_x;
                let nearest_y = y0 + projection * delta_y;
                let distance = ((sample_x - nearest_x).powi(2) + (sample_y - nearest_y).powi(2)).sqrt() - half;
                let coverage = (0.5 - distance).clamp(0.0, 1.0);
                if coverage > 0.0 {
                    self.blend_pixel(column, row, scale_premultiplied(premultiplied, coverage));
                }
            }
        }
    }

    /// Downward-pointing triangle (the speech-bubble tail): flat top edge from
    /// (`x`, `y`) spanning `width`, apex `height` below its middle. Edge pixels are
    /// blended fractionally so the tail does not look jagged.
    pub fn fill_triangle_down(&mut self, x: f32, y: f32, width: f32, height: f32, color: Color) {
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        let premultiplied = premultiply(color);
        let center_x = x + width / 2.0;
        let first_row = y.floor() as i32;
        let last_row = (y + height).ceil() as i32;
        for row in first_row.max(0)..last_row.min(self.height as i32) {
            let sample_y = row as f32 + 0.5;
            if sample_y < y || sample_y > y + height {
                continue;
            }
            let progress = ((sample_y - y) / height).clamp(0.0, 1.0);
            let half_span = (width / 2.0) * (1.0 - progress);
            let left = center_x - half_span;
            let right = center_x + half_span;
            for column in (left.floor() as i32).max(0)..(right.ceil() as i32).min(self.width as i32) {
                let pixel_left = column as f32;
                let pixel_right = pixel_left + 1.0;
                let coverage = (right.min(pixel_right) - left.max(pixel_left)).clamp(0.0, 1.0);
                if coverage > 0.0 {
                    self.blend_pixel(column, row, scale_premultiplied(premultiplied, coverage));
                }
            }
        }
    }

    /// Composites a straight-alpha RGBA8 image (row-major, top-down) over the canvas.
    pub fn blit_rgba(&mut self, image: &[u8], image_width: u32, image_height: u32, x: i32, y: i32) {
        for row in 0..image_height as i32 {
            for column in 0..image_width as i32 {
                let offset = (row as usize * image_width as usize + column as usize) * 4;
                let alpha = image[offset + 3];
                if alpha == 0 {
                    continue;
                }
                let color = Color::rgba(image[offset], image[offset + 1], image[offset + 2], alpha);
                self.blend_pixel(x + column, y + row, premultiply(color));
            }
        }
    }

    /// Draws one line of anti-aliased text; (`x`, `y`) is the top-left corner of the line
    /// box (the rasterizer places the baseline at `y + ascent`). Returns the advance width.
    pub fn draw_text(&mut self, rasterizer: &mut TextRasterizer, text: &str, x: f32, y: f32, color: Color) -> f32 {
        let premultiplied = premultiply(color);
        rasterizer.rasterize_line(text, x, y, |pixel_x, pixel_y, coverage| {
            if coverage > 0.0 {
                self.blend_pixel(pixel_x, pixel_y, scale_premultiplied(premultiplied, coverage));
            }
        })
    }
}

/// Signed distance from a point (relative to the centre) to a rounded rectangle with the
/// given half extents and corner radius; negative inside.
fn rounded_rect_signed_distance(dx: f32, dy: f32, half_width: f32, half_height: f32, radius: f32) -> f32 {
    let qx = dx.abs() - (half_width - radius);
    let qy = dy.abs() - (half_height - radius);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let inside = qx.max(qy).min(0.0);
    outside + inside - radius
}

/// Greedy word wrap into at most `max_lines` lines no wider than `max_width` according to
/// `measure` (a width function in any unit — pixels for real fonts, character counts in
/// tests). Words wider than a line are hard-broken by character. When the text does not
/// fit, the last line is truncated and ends with `…`. Always returns at least one line.
pub fn wrap_text(text: &str, max_width: f32, max_lines: usize, mut measure: impl FnMut(&str) -> f32) -> Vec<String> {
    const ELLIPSIS: &str = "…";
    let max_lines = max_lines.max(1);
    let mut fits = |candidate: &str| measure(candidate) <= max_width;

    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let candidate = if current.is_empty() { word.to_string() } else { format!("{current} {word}") };
        if fits(&candidate) {
            current = candidate;
            continue;
        }
        if !current.is_empty() {
            lines.push(std::mem::take(&mut current));
        }
        if fits(word) {
            current = word.to_string();
        } else {
            let mut pieces = hard_break_word(word, &mut fits);
            current = pieces.pop().unwrap_or_default();
            lines.extend(pieces);
        }
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }

    if lines.len() > max_lines {
        lines.truncate(max_lines);
        // Trim the last line until it fits together with the ellipsis.
        let last = lines.last_mut().expect("max_lines >= 1");
        let mut characters: Vec<char> = last.chars().collect();
        loop {
            let mut candidate: String = characters.iter().collect::<String>().trim_end().to_string();
            candidate.push_str(ELLIPSIS);
            if fits(&candidate) || characters.is_empty() {
                *last = candidate;
                break;
            }
            characters.pop();
        }
    }
    lines
}

/// Splits a word wider than a line into the largest chunks that fit (at least one char each).
fn hard_break_word(word: &str, fits: &mut impl FnMut(&str) -> bool) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut current = String::new();
    for character in word.chars() {
        let mut candidate = current.clone();
        candidate.push(character);
        if !current.is_empty() && !fits(&candidate) {
            pieces.push(std::mem::take(&mut current));
            current.push(character);
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        pieces.push(current);
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    fn char_count(text: &str) -> f32 {
        text.chars().count() as f32
    }

    #[test]
    fn premultiply_round_trips_through_unpremultiply() {
        let color = Color::rgba(200, 100, 50, 128);
        let pixel = premultiply(color);
        assert_eq!(pixel >> 24, 128);
        let back = unpremultiply(pixel);
        assert!((back.red as i32 - 200).abs() <= 1, "{back:?}");
        assert!((back.green as i32 - 100).abs() <= 1, "{back:?}");
        assert!((back.blue as i32 - 50).abs() <= 1, "{back:?}");
        assert_eq!(back.alpha, 128);
        assert_eq!(premultiply(Color::rgb(0x12, 0x34, 0x56)), 0xFF12_3456);
        assert_eq!(premultiply(Color::TRANSPARENT), 0);
    }

    #[test]
    fn blend_over_transparent_keeps_source_exactly() {
        let source = premultiply(Color::rgba(255, 0, 0, 100));
        assert_eq!(blend_premultiplied(0, source), source);
    }

    #[test]
    fn blend_opaque_over_anything_replaces() {
        let destination = premultiply(Color::rgba(0, 255, 0, 200));
        let source = premultiply(Color::rgb(10, 20, 30));
        assert_eq!(blend_premultiplied(destination, source), source);
    }

    #[test]
    fn blend_half_white_over_opaque_black_is_mid_gray() {
        let destination = premultiply(Color::rgb(0, 0, 0));
        let mixed = unpremultiply(blend_premultiplied(destination, premultiply(Color::rgba(255, 255, 255, 128))));
        assert_eq!(mixed.alpha, 255);
        assert!((127..=129).contains(&mixed.red), "{mixed:?}");
    }

    #[test]
    fn blend_two_translucent_layers_accumulates_alpha() {
        let layer = premultiply(Color::rgba(255, 255, 255, 128));
        let combined = blend_premultiplied(layer, layer);
        // 0.5 + 0.5 * 0.5 = 0.75 → 191.
        let alpha = combined >> 24;
        assert!((190..=192).contains(&alpha), "alpha {alpha}");
    }

    #[test]
    fn new_canvas_is_fully_transparent_and_fill_rect_clips() {
        let mut canvas = Canvas::new(4, 4);
        assert!(canvas.pixels.iter().all(|&pixel| pixel == 0));
        canvas.fill_rect(-2, -2, 10, 10, Color::rgb(1, 2, 3));
        assert!(canvas.pixels.iter().all(|&pixel| pixel == premultiply(Color::rgb(1, 2, 3))));
        canvas.fill_rect(3, 3, 5, 5, Color::rgb(9, 9, 9));
        assert_eq!(canvas.color_at(3, 3), Color::rgb(9, 9, 9));
        assert_eq!(canvas.color_at(2, 2), Color::rgb(1, 2, 3));
        assert_eq!(canvas.color_at(-1, 0), Color::TRANSPARENT);
    }

    #[test]
    fn translucent_fill_over_transparent_keeps_alpha() {
        let mut canvas = Canvas::new(2, 2);
        canvas.fill_rect(0, 0, 2, 2, Color::rgba(40, 40, 44, 200));
        let color = canvas.color_at(0, 0);
        assert_eq!(color.alpha, 200);
        assert!((color.red as i32 - 40).abs() <= 1, "{color:?}");
    }

    #[test]
    fn rounded_rect_keeps_corners_empty_and_edges_smooth() {
        let mut canvas = Canvas::new(20, 20);
        canvas.fill_rounded_rect(0.0, 0.0, 20.0, 20.0, 6.0, Color::rgb(255, 0, 0));
        assert_eq!(canvas.pixel_at(0, 0), 0);
        assert_eq!(canvas.pixel_at(19, 19), 0);
        assert_eq!(canvas.color_at(10, 10), Color::rgb(255, 0, 0));
        assert_eq!(canvas.color_at(10, 0), Color::rgb(255, 0, 0));
        assert_eq!(canvas.color_at(0, 10), Color::rgb(255, 0, 0));
        // Somewhere along the corner arc there must be a partially covered pixel.
        let has_partial = (0..6).any(|offset| {
            let alpha = canvas.color_at(offset, 5 - offset).alpha;
            alpha > 0 && alpha < 255
        });
        assert!(has_partial);
    }

    #[test]
    fn stroke_rounded_rect_leaves_the_interior_empty() {
        let mut canvas = Canvas::new(20, 20);
        canvas.stroke_rounded_rect(0.0, 0.0, 20.0, 20.0, 4.0, 1.0, Color::WHITE);
        assert_eq!(canvas.pixel_at(10, 10), 0);
        assert_eq!(canvas.color_at(10, 0), Color::WHITE);
        assert_eq!(canvas.color_at(0, 10), Color::WHITE);
        assert_eq!(canvas.pixel_at(10, 2), 0);
    }

    #[test]
    fn circle_and_line_cover_expected_pixels() {
        let mut canvas = Canvas::new(11, 11);
        canvas.fill_circle(5.5, 5.5, 4.0, Color::WHITE);
        assert_eq!(canvas.color_at(5, 5), Color::WHITE);
        assert_eq!(canvas.pixel_at(0, 0), 0);
        let mut line_canvas = Canvas::new(11, 11);
        line_canvas.draw_line(1.0, 5.5, 10.0, 5.5, 2.0, Color::WHITE);
        assert_eq!(line_canvas.color_at(5, 5).alpha, 255);
        assert_eq!(line_canvas.pixel_at(5, 0), 0);
    }

    #[test]
    fn triangle_down_narrows_towards_the_apex() {
        let mut canvas = Canvas::new(12, 6);
        canvas.fill_triangle_down(0.0, 0.0, 12.0, 6.0, Color::WHITE);
        assert_eq!(canvas.color_at(1, 0).alpha, 255);
        assert_eq!(canvas.pixel_at(1, 5), 0);
        assert!(canvas.color_at(5, 5).alpha > 0);
    }

    #[test]
    fn blit_rgba_respects_alpha_and_clips() {
        let mut canvas = Canvas::new(2, 1);
        // 2x1 image: opaque red, transparent green.
        let image = [255, 0, 0, 255, 0, 255, 0, 0];
        canvas.blit_rgba(&image, 2, 1, 0, 0);
        assert_eq!(canvas.color_at(0, 0), Color::rgb(255, 0, 0));
        assert_eq!(canvas.pixel_at(1, 0), 0);
        // Off-canvas blit must not panic.
        canvas.blit_rgba(&image, 2, 1, -1, 5);
    }

    #[test]
    fn wrap_text_simple_fit() {
        assert_eq!(wrap_text("hello world", 20.0, 2, char_count), vec!["hello world"]);
        assert_eq!(wrap_text("hello world", 6.0, 3, char_count), vec!["hello", "world"]);
    }

    #[test]
    fn wrap_text_truncates_with_ellipsis() {
        let lines = wrap_text("one two three four five six seven", 8.0, 2, char_count);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].ends_with('…'), "got {lines:?}");
        assert!(char_count(&lines[1]) <= 8.0);
    }

    #[test]
    fn wrap_text_hard_breaks_long_words() {
        let lines = wrap_text("abcdefghijklmnop", 5.0, 4, char_count);
        assert_eq!(lines, vec!["abcde", "fghij", "klmno", "p"]);
    }

    #[test]
    fn wrap_text_korean_by_measure() {
        let lines = wrap_text("승인할까요? git push", 10.0, 2, char_count);
        assert_eq!(lines, vec!["승인할까요? git", "push"]);
    }

    #[test]
    fn wrap_text_empty_input() {
        assert_eq!(wrap_text("", 10.0, 2, char_count), vec![String::new()]);
    }
}
