//! Anti-aliased system-font text for the overlay (speech bubble, session label, badge
//! glyphs), rasterised in pure Rust with `ab_glyph`.
//!
//! A `TextRasterizer` owns a *font chain*: the preferred UI face first (SF on macOS,
//! optionally the rounded cut for the label), then Latin and Hangul/CJK fallbacks. Each
//! character is looked up along the chain, so mixed text like "승인할까요? git push"
//! renders every glyph. Rasterised glyphs are cached per (font, glyph, pixel size).
//!
//! Font discovery is the only platform-specific corner (`platform_font_files`); Windows
//! and Linux ports supply their own file lists there and nothing else changes.

use std::collections::HashMap;
use std::path::PathBuf;

use ab_glyph::{point, Font, FontVec, GlyphId, PxScale, ScaleFont, VariableFont};

/// Which UI face to load; mirrors the SwiftUI `Font.system(size:weight:design:)` calls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontStyle {
    /// CSS-style weight (400 regular, 500 medium, 600 semibold); applied through the
    /// variable font's `wght` axis when the face has one.
    pub weight: f32,
    /// Prefer the rounded design (SF Rounded on macOS) like `.design(.rounded)`.
    pub is_rounded: bool,
}

impl FontStyle {
    pub const BUBBLE: FontStyle = FontStyle { weight: 500.0, is_rounded: false };
    pub const LABEL: FontStyle = FontStyle { weight: 600.0, is_rounded: true };
    pub const BADGE: FontStyle = FontStyle { weight: 700.0, is_rounded: false };
}

/// One font file to load: path plus face index inside a `.ttc` collection (0 for `.ttf`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontFile {
    pub path: PathBuf,
    pub collection_index: u32,
}

/// The font chain for `style`, most preferred first. Missing files are skipped at load
/// time, so this list may name fonts that only exist on some OS versions.
#[cfg(target_os = "macos")]
pub fn platform_font_files(style: FontStyle) -> Vec<FontFile> {
    let system_font = |name: &str, collection_index: u32| FontFile {
        path: PathBuf::from("/System/Library/Fonts").join(name),
        collection_index,
    };
    let mut files = Vec::new();
    if style.is_rounded {
        files.push(system_font("SFNSRounded.ttf", 0));
    }
    files.push(system_font("SFNS.ttf", 0));
    files.push(system_font("Helvetica.ttc", 0));
    // Hangul (and Kana/Han) fallback — the same face AppKit picks for Korean UI text.
    files.push(system_font("AppleSDGothicNeo.ttc", 0));
    files
}

#[cfg(not(target_os = "macos"))]
pub fn platform_font_files(_style: FontStyle) -> Vec<FontFile> {
    Vec::new()
}

/// A rasterised glyph bitmap in the glyph cache. `left`/`top` are the bitmap's offset from
/// the pen position (top relative to the baseline, positive downwards).
struct CachedGlyph {
    left: i32,
    top: i32,
    width: u32,
    height: u32,
    coverage: Vec<f32>,
    advance: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphCacheKey {
    font_index: usize,
    glyph_id: u16,
    pixel_size_bits: u32,
}

/// Renders single lines of text at one pixel size at a time (`set_pixel_size`).
pub struct TextRasterizer {
    fonts: Vec<FontVec>,
    pixel_size: f32,
    cache: HashMap<GlyphCacheKey, CachedGlyph>,
}

impl TextRasterizer {
    /// Loads the platform font chain for `style`. Never fails: an empty chain draws
    /// nothing and measures every glyph as `0.6 × size` so layout stays sane.
    pub fn load_system(style: FontStyle) -> TextRasterizer {
        let mut fonts = Vec::new();
        for file in platform_font_files(style) {
            match Self::load_font_file(&file, style.weight) {
                Some(font) => fonts.push(font),
                None => super::log(&format!("text: could not load font {}", file.path.display())),
            }
        }
        if fonts.is_empty() {
            super::log("text: no system font could be loaded; text will not render");
        }
        TextRasterizer { fonts, pixel_size: 12.0, cache: HashMap::new() }
    }

    /// A rasterizer over caller-supplied font bytes (tests, embedded fonts).
    #[cfg(test)]
    pub fn from_font_data(font_files: Vec<Vec<u8>>, weight: f32) -> TextRasterizer {
        let fonts = font_files
            .into_iter()
            .filter_map(|data| {
                let mut font = FontVec::try_from_vec(data).ok()?;
                Self::apply_ui_text_variations(&mut font, weight);
                Some(font)
            })
            .collect();
        TextRasterizer { fonts, pixel_size: 12.0, cache: HashMap::new() }
    }

    fn load_font_file(file: &FontFile, weight: f32) -> Option<FontVec> {
        let data = std::fs::read(&file.path).ok()?;
        let mut font = FontVec::try_from_vec_and_index(data, file.collection_index).ok()?;
        Self::apply_ui_text_variations(&mut font, weight);
        Some(font)
    }

    /// Selects the weight and the small-text optical size on variable faces (SF's `opsz`
    /// axis defaults to the 28 pt Display cut; the overlay's text is 7.5–11.5 pt, so the
    /// Text cut is what AppKit would use). Static faces have no axes and are left alone.
    fn apply_ui_text_variations(font: &mut FontVec, weight: f32) {
        const OVERLAY_TEXT_POINT_SIZE: f32 = 11.5;
        let optical_size_axis = font.variations().into_iter().find(|axis| &axis.tag == b"opsz");
        font.set_variation(b"wght", weight);
        if let Some(axis) = optical_size_axis {
            font.set_variation(b"opsz", OVERLAY_TEXT_POINT_SIZE.clamp(axis.min_value, axis.max_value));
        }
    }

    #[cfg(test)]
    pub fn has_fonts(&self) -> bool {
        !self.fonts.is_empty()
    }

    /// The em size in pixels used by all following measure/draw calls
    /// (`point size × window scale factor`).
    pub fn set_pixel_size(&mut self, pixel_size: f32) {
        self.pixel_size = pixel_size.max(1.0);
    }

    fn scale_for(&self, font_index: usize) -> PxScale {
        let font = &self.fonts[font_index];
        // ab_glyph's PxScale is the ascent-to-descent height, not the em size.
        let units_per_em = font.units_per_em().unwrap_or(1000.0);
        PxScale::from(self.pixel_size * font.height_unscaled() / units_per_em)
    }

    /// Distance from the top of the line box to the baseline (primary font).
    pub fn ascent(&self) -> f32 {
        match self.fonts.first() {
            Some(font) => font.as_scaled(self.scale_for(0)).ascent(),
            None => self.pixel_size * 0.8,
        }
    }

    /// Ascent + |descent| of the primary font: the height of one line box.
    pub fn line_height(&self) -> f32 {
        match self.fonts.first() {
            Some(font) => {
                let scaled = font.as_scaled(self.scale_for(0));
                scaled.ascent() - scaled.descent()
            }
            None => self.pixel_size * 1.2,
        }
    }

    /// Advance width of `text` at the current pixel size.
    pub fn measure(&mut self, text: &str) -> f32 {
        self.rasterize_line(text, 0.0, 0.0, |_, _, _| {})
    }

    /// First font in the chain that has a real glyph for `character`.
    fn resolve_glyph(&self, character: char) -> Option<(usize, GlyphId)> {
        self.fonts.iter().enumerate().find_map(|(font_index, font)| {
            let glyph_id = font.glyph_id(character);
            (glyph_id.0 != 0).then_some((font_index, glyph_id))
        })
    }

    fn cached_glyph(&mut self, font_index: usize, glyph_id: GlyphId) -> &CachedGlyph {
        let key = GlyphCacheKey { font_index, glyph_id: glyph_id.0, pixel_size_bits: self.pixel_size.to_bits() };
        if !self.cache.contains_key(&key) {
            let scale = self.scale_for(font_index);
            let scaled_font = self.fonts[font_index].as_scaled(scale);
            let advance = scaled_font.h_advance(glyph_id);
            let glyph = glyph_id.with_scale_and_position(scale, point(0.0, 0.0));
            let rendered = match scaled_font.outline_glyph(glyph) {
                Some(outlined) => {
                    let bounds = outlined.px_bounds();
                    let width = (bounds.width().ceil().max(0.0)) as u32;
                    let height = (bounds.height().ceil().max(0.0)) as u32;
                    let mut coverage = vec![0.0f32; width as usize * height as usize];
                    outlined.draw(|x, y, value| {
                        if x < width && y < height {
                            coverage[y as usize * width as usize + x as usize] = value.clamp(0.0, 1.0);
                        }
                    });
                    CachedGlyph {
                        left: bounds.min.x.floor() as i32,
                        top: bounds.min.y.floor() as i32,
                        width,
                        height,
                        coverage,
                        advance,
                    }
                }
                None => CachedGlyph { left: 0, top: 0, width: 0, height: 0, coverage: Vec::new(), advance },
            };
            self.cache.insert(key, rendered);
        }
        &self.cache[&key]
    }

    /// Lays out `text` on one line whose box's top-left corner is (`x`, `y`) and calls
    /// `sink(pixel_x, pixel_y, coverage)` for every glyph pixel. Returns the total advance.
    /// Characters missing from every font advance by 0.6 em and draw nothing.
    pub fn rasterize_line(&mut self, text: &str, x: f32, y: f32, mut sink: impl FnMut(i32, i32, f32)) -> f32 {
        let baseline_y = y + self.ascent();
        let missing_advance = self.pixel_size * 0.6;
        let mut pen_x = x;
        let mut previous: Option<(usize, GlyphId)> = None;
        for character in text.chars() {
            let Some((font_index, glyph_id)) = self.resolve_glyph(character) else {
                pen_x += missing_advance;
                previous = None;
                continue;
            };
            if let Some((previous_font, previous_glyph)) = previous {
                if previous_font == font_index {
                    let scale = self.scale_for(font_index);
                    pen_x += self.fonts[font_index].as_scaled(scale).kern(previous_glyph, glyph_id);
                }
            }
            let origin_x = pen_x.round() as i32;
            let origin_y = baseline_y.round() as i32;
            let glyph = self.cached_glyph(font_index, glyph_id);
            for row in 0..glyph.height {
                for column in 0..glyph.width {
                    let coverage = glyph.coverage[row as usize * glyph.width as usize + column as usize];
                    if coverage > 0.0 {
                        sink(origin_x + glyph.left + column as i32, origin_y + glyph.top + row as i32, coverage);
                    }
                }
            }
            pen_x += glyph.advance;
            previous = Some((font_index, glyph_id));
        }
        pen_x - x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_chain_measures_and_draws_nothing_but_keeps_layout_sane() {
        let mut rasterizer = TextRasterizer::from_font_data(Vec::new(), 400.0);
        rasterizer.set_pixel_size(10.0);
        assert!(!rasterizer.has_fonts());
        assert!((rasterizer.measure("abc") - 18.0).abs() < 0.01);
        assert!(rasterizer.line_height() > 0.0);
        let mut pixel_count = 0;
        rasterizer.rasterize_line("abc", 0.0, 0.0, |_, _, _| pixel_count += 1);
        assert_eq!(pixel_count, 0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn system_fonts_render_latin_and_hangul() {
        let mut rasterizer = TextRasterizer::load_system(FontStyle::BUBBLE);
        assert!(rasterizer.has_fonts(), "system font chain is empty");
        rasterizer.set_pixel_size(24.0);
        let latin_width = rasterizer.measure("git push");
        let hangul_width = rasterizer.measure("승인할까요?");
        assert!(latin_width > 40.0, "latin width {latin_width}");
        assert!(hangul_width > 40.0, "hangul width {hangul_width}");
        let mut painted_pixels = 0;
        rasterizer.rasterize_line("한", 0.0, 0.0, |_, _, coverage| {
            if coverage > 0.5 {
                painted_pixels += 1;
            }
        });
        assert!(painted_pixels > 20, "Hangul glyph painted {painted_pixels} pixels");
        // Missing glyphs advance without panicking; the cache is reused across calls.
        let cache_size_before = rasterizer.cache.len();
        rasterizer.measure("승인할까요?");
        assert_eq!(rasterizer.cache.len(), cache_size_before);
    }
}
