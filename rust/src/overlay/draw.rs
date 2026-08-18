//! Software framebuffer painting for the overlay. The canvas is a `Vec<u32>` in
//! softbuffer's `0RGB` format (`0x00RRGGBB`, alpha byte ignored by softbuffer).
//!
//! NOTE (transparency fallback): softbuffer 0.4 cannot deliver per-pixel window
//! transparency on macOS — the buffer format has no alpha channel. The overlay therefore
//! draws everything onto an opaque dark "card" background instead of a transparent
//! window. This is a deliberate, documented deviation from the Swift overlay's look;
//! see the report in rust/README.md. Functionality (states, messages, gauge, tray menu)
//! is unaffected.

use super::font;

pub fn rgb(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

/// Straight-alpha source-over of an RGBA color onto an opaque 0RGB pixel.
pub fn blend_over(dst: u32, r: u8, g: u8, b: u8, a: u8) -> u32 {
    match a {
        0 => dst,
        255 => rgb(r, g, b),
        _ => {
            let alpha = a as u32;
            let inverse = 255 - alpha;
            let dr = (dst >> 16) & 0xFF;
            let dg = (dst >> 8) & 0xFF;
            let db = dst & 0xFF;
            let out_r = (r as u32 * alpha + dr * inverse + 127) / 255;
            let out_g = (g as u32 * alpha + dg * inverse + 127) / 255;
            let out_b = (b as u32 * alpha + db * inverse + 127) / 255;
            (out_r << 16) | (out_g << 8) | out_b
        }
    }
}

pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
}

impl Canvas {
    pub fn new(width: u32, height: u32) -> Canvas {
        Canvas {
            width,
            height,
            pixels: vec![0u32; width as usize * height as usize],
        }
    }

    pub fn fill(&mut self, color: u32) {
        self.pixels.fill(color);
    }

    fn set(&mut self, x: i32, y: i32, color: u32) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        self.pixels[y as usize * self.width as usize + x as usize] = color;
    }

    fn get(&self, x: i32, y: i32) -> u32 {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return 0;
        }
        self.pixels[y as usize * self.width as usize + x as usize]
    }

    /// Clipped rectangle fill.
    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: u32) {
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + w).min(self.width as i32);
        let y1 = (y + h).min(self.height as i32);
        for py in y0..y1 {
            for px in x0..x1 {
                self.pixels[py as usize * self.width as usize + px as usize] = color;
            }
        }
    }

    /// Rectangle with quarter-circle corners (radius clamped to half the short side).
    pub fn fill_rounded_rect(&mut self, x: i32, y: i32, w: i32, h: i32, radius: i32, color: u32) {
        if w <= 0 || h <= 0 {
            return;
        }
        let radius = radius.max(0).min(w / 2).min(h / 2);
        if radius == 0 {
            self.fill_rect(x, y, w, h, color);
            return;
        }
        // Middle band, then top/bottom bands with per-row inset from the circle equation.
        self.fill_rect(x, y + radius, w, h - 2 * radius, color);
        for row in 0..radius {
            let dy = radius - row; // distance from the corner circle's centre row
            let dx = ((radius * radius - dy * dy) as f64).sqrt() as i32;
            let inset = radius - dx;
            self.fill_rect(x + inset, y + row, w - 2 * inset, 1, color);
            self.fill_rect(x + inset, y + h - 1 - row, w - 2 * inset, 1, color);
        }
    }

    pub fn fill_circle(&mut self, cx: i32, cy: i32, radius: i32, color: u32) {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx * dx + dy * dy <= radius * radius {
                    self.set(cx + dx, cy + dy, color);
                }
            }
        }
    }

    /// Thick line drawn as a run of filled squares (enough for badge glyphs).
    pub fn draw_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, thickness: i32, color: u32) {
        let steps = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
        let half = (thickness / 2).max(0);
        for step in 0..=steps {
            let x = x0 + (x1 - x0) * step / steps;
            let y = y0 + (y1 - y0) * step / steps;
            self.fill_rect(x - half, y - half, thickness.max(1), thickness.max(1), color);
        }
    }

    /// Composites a straight-alpha RGBA8 image (row-major, top-down) over the canvas.
    pub fn blit_rgba(&mut self, image: &[u8], image_width: u32, image_height: u32, x: i32, y: i32) {
        for row in 0..image_height as i32 {
            for column in 0..image_width as i32 {
                let offset = (row as usize * image_width as usize + column as usize) * 4;
                let a = image[offset + 3];
                if a == 0 {
                    continue;
                }
                let dst = self.get(x + column, y + row);
                let blended = blend_over(dst, image[offset], image[offset + 1], image[offset + 2], a);
                self.set(x + column, y + row, blended);
            }
        }
    }

    /// Draws `text` with the embedded 8×8 font at integer `scale`; returns the width drawn.
    /// Non-ASCII characters render as '?' (see `font::glyph`).
    pub fn draw_text(&mut self, text: &str, x: i32, y: i32, scale: i32, color: u32) -> i32 {
        let scale = scale.max(1);
        let mut cursor = x;
        for character in text.chars() {
            let bitmap = font::glyph(character);
            for gy in 0..font::GLYPH_HEIGHT {
                for gx in 0..font::GLYPH_WIDTH {
                    if font::glyph_pixel(bitmap, gx, gy) {
                        self.fill_rect(
                            cursor + gx as i32 * scale,
                            y + gy as i32 * scale,
                            scale,
                            scale,
                            color,
                        );
                    }
                }
            }
            cursor += font::GLYPH_WIDTH as i32 * scale;
        }
        cursor - x
    }
}

pub fn text_width(text: &str, scale: i32) -> i32 {
    text.chars().count() as i32 * font::GLYPH_WIDTH as i32 * scale.max(1)
}

/// Greedy word wrap to at most `max_lines` lines of `max_chars` characters; the last
/// line is truncated with "..." when the text does not fit. Words longer than a line
/// are hard-broken. Returns at least one (possibly empty) line for non-empty input.
pub fn wrap_text(text: &str, max_chars: usize, max_lines: usize) -> Vec<String> {
    let max_chars = max_chars.max(4);
    let max_lines = max_lines.max(1);

    // First: break into as many lines as needed (greedy word wrap, overlong words hard-broken).
    let mut all_lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;
    for word in text.split_whitespace() {
        let mut pieces: Vec<String> = Vec::new();
        let mut chars: Vec<char> = word.chars().collect();
        while chars.len() > max_chars {
            pieces.push(chars.drain(..max_chars).collect());
        }
        pieces.push(chars.into_iter().collect());
        for (index, piece) in pieces.iter().enumerate() {
            let piece_len = piece.chars().count();
            let hard_broken = index + 1 < pieces.len() || pieces.len() > 1 && index > 0;
            let separator = if current_len == 0 { 0 } else { 1 };
            let fits = current_len + separator + piece_len <= max_chars;
            if fits && !(hard_broken && index > 0) {
                if separator == 1 {
                    current.push(' ');
                }
                current.push_str(piece);
                current_len += separator + piece_len;
            } else {
                if current_len > 0 {
                    all_lines.push(std::mem::take(&mut current));
                }
                current.push_str(piece);
                current_len = piece_len;
            }
            if index + 1 < pieces.len() {
                // A full-width chunk of a hard-broken word always ends its line.
                all_lines.push(std::mem::take(&mut current));
                current_len = 0;
            }
        }
    }
    if !current.is_empty() || all_lines.is_empty() {
        all_lines.push(current);
    }

    // Then: cap at max_lines, marking truncation with "...".
    if all_lines.len() > max_lines {
        all_lines.truncate(max_lines);
        if let Some(last) = all_lines.last_mut() {
            let keep = max_chars.saturating_sub(3);
            let mut kept: String = last.chars().take(keep).collect();
            kept.push_str("...");
            *last = kept;
        }
    }
    all_lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_packs_0rgb() {
        assert_eq!(rgb(0x12, 0x34, 0x56), 0x0012_3456);
        assert_eq!(rgb(255, 255, 255), 0x00FF_FFFF);
    }

    #[test]
    fn blend_over_extremes_and_midpoint() {
        let dst = rgb(0, 0, 0);
        assert_eq!(blend_over(dst, 10, 20, 30, 0), dst);
        assert_eq!(blend_over(dst, 10, 20, 30, 255), rgb(10, 20, 30));
        // 50% white over black ≈ mid gray.
        let half = blend_over(dst, 255, 255, 255, 128);
        let r = (half >> 16) & 0xFF;
        assert!((127..=129).contains(&r), "got {r}");
    }

    #[test]
    fn fill_rect_clips() {
        let mut canvas = Canvas::new(4, 4);
        canvas.fill_rect(-2, -2, 10, 10, rgb(1, 2, 3));
        assert!(canvas.pixels.iter().all(|&pixel| pixel == rgb(1, 2, 3)));
        canvas.fill_rect(3, 3, 5, 5, rgb(9, 9, 9));
        assert_eq!(canvas.get(3, 3), rgb(9, 9, 9));
        assert_eq!(canvas.get(2, 2), rgb(1, 2, 3));
    }

    #[test]
    fn rounded_rect_keeps_corners_empty() {
        let mut canvas = Canvas::new(20, 20);
        canvas.fill_rounded_rect(0, 0, 20, 20, 6, rgb(255, 0, 0));
        // Extreme corner pixel stays background, centre and edge midpoints filled.
        assert_eq!(canvas.get(0, 0), 0);
        assert_eq!(canvas.get(19, 19), 0);
        assert_eq!(canvas.get(10, 10), rgb(255, 0, 0));
        assert_eq!(canvas.get(10, 0), rgb(255, 0, 0));
        assert_eq!(canvas.get(0, 10), rgb(255, 0, 0));
    }

    #[test]
    fn blit_rgba_respects_alpha_and_clips() {
        let mut canvas = Canvas::new(2, 1);
        canvas.fill(rgb(0, 0, 0));
        // 2x1 image: opaque red, transparent green.
        let image = [255, 0, 0, 255, 0, 255, 0, 0];
        canvas.blit_rgba(&image, 2, 1, 0, 0);
        assert_eq!(canvas.get(0, 0), rgb(255, 0, 0));
        assert_eq!(canvas.get(1, 0), rgb(0, 0, 0));
        // Off-canvas blit must not panic.
        canvas.blit_rgba(&image, 2, 1, -1, 5);
    }

    #[test]
    fn draw_text_width_and_pixels() {
        let mut canvas = Canvas::new(64, 16);
        let width = canvas.draw_text("Hi", 0, 0, 1, rgb(255, 255, 255));
        assert_eq!(width, 16);
        assert_eq!(text_width("Hi", 1), 16);
        assert_eq!(text_width("Hi", 2), 32);
        assert!(canvas.pixels.iter().any(|&pixel| pixel == rgb(255, 255, 255)));
    }

    #[test]
    fn wrap_text_simple_fit() {
        assert_eq!(wrap_text("hello world", 20, 2), vec!["hello world"]);
        assert_eq!(wrap_text("hello world", 6, 3), vec!["hello", "world"]);
    }

    #[test]
    fn wrap_text_truncates_with_ellipsis() {
        let lines = wrap_text("one two three four five six seven", 8, 2);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].ends_with("..."), "got {lines:?}");
        assert!(lines[1].chars().count() <= 8);
    }

    #[test]
    fn wrap_text_hard_breaks_long_words() {
        let lines = wrap_text("abcdefghijklmnop", 5, 4);
        assert_eq!(lines[0], "abcde");
        assert_eq!(lines[1], "fghij");
        assert_eq!(lines[2], "klmno");
        assert_eq!(lines[3], "p");
    }

    #[test]
    fn wrap_text_empty_input() {
        assert_eq!(wrap_text("", 10, 2), vec![String::new()]);
    }
}
