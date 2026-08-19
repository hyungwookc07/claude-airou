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

/// The horizontal, default-track row of an AAT `trak` table: per point size, extra advance
/// per glyph in font units. CoreText applies it automatically to the system fonts, which
/// is where SF's generous small-size letter spacing comes from; without it our labels
/// come out ~10 % narrower than AppKit's.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackingTable {
    /// Ascending point sizes.
    sizes: Vec<f32>,
    /// Tracking in font units at each size (same length as `sizes`).
    values: Vec<f32>,
}

impl TrackingTable {
    /// Parses the `trak` table out of a raw TrueType/OpenType font file (single face, not a
    /// `.ttc`); None when the font has none or the table is malformed.
    pub fn parse(font_data: &[u8]) -> Option<TrackingTable> {
        let read_u16 = |offset: usize| -> Option<u16> {
            font_data.get(offset..offset + 2).map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
        };
        let read_u32 = |offset: usize| -> Option<u32> {
            font_data
                .get(offset..offset + 4)
                .map(|bytes| u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        };
        let table_count = read_u16(4)? as usize;
        let mut table_offset = None;
        for index in 0..table_count {
            let record = 12 + 16 * index;
            if font_data.get(record..record + 4)? == b"trak" {
                table_offset = Some(read_u32(record + 8)? as usize);
                break;
            }
        }
        let table = table_offset?;
        let horizontal_offset = read_u16(table + 6)? as usize;
        if horizontal_offset == 0 {
            return None;
        }
        let horizontal = table + horizontal_offset;
        let track_count = read_u16(horizontal)? as usize;
        let size_count = read_u16(horizontal + 2)? as usize;
        let size_table_offset = table + read_u32(horizontal + 4)? as usize;
        let mut sizes = Vec::with_capacity(size_count);
        for index in 0..size_count {
            // Sizes are 16.16 fixed-point points.
            sizes.push(read_u32(size_table_offset + 4 * index)? as i32 as f32 / 65536.0);
        }
        // Prefer the default track (0.0); fall back to the first one.
        let mut chosen_values_offset = None;
        for index in 0..track_count {
            let entry = horizontal + 8 + 8 * index;
            let track = read_u32(entry)? as i32;
            let values_offset = read_u16(entry + 6)? as usize;
            if track == 0 || chosen_values_offset.is_none() {
                chosen_values_offset = Some(values_offset);
            }
            if track == 0 {
                break;
            }
        }
        let values_offset = table + chosen_values_offset?;
        let mut values = Vec::with_capacity(size_count);
        for index in 0..size_count {
            values.push(read_u16(values_offset + 2 * index)? as i16 as f32);
        }
        if sizes.is_empty() || sizes.len() != values.len() {
            return None;
        }
        Some(TrackingTable { sizes, values })
    }

    /// Tracking in font units at `point_size` (linear interpolation, clamped at the ends).
    pub fn tracking_units(&self, point_size: f32) -> f32 {
        if point_size <= self.sizes[0] {
            return self.values[0];
        }
        for index in 1..self.sizes.len() {
            if point_size <= self.sizes[index] {
                let span = self.sizes[index] - self.sizes[index - 1];
                if span <= 0.0 {
                    return self.values[index];
                }
                let progress = (point_size - self.sizes[index - 1]) / span;
                return self.values[index - 1] + (self.values[index] - self.values[index - 1]) * progress;
            }
        }
        *self.values.last().unwrap_or(&0.0)
    }
}

/// Renders single lines of text at one size at a time (`set_size`).
pub struct TextRasterizer {
    fonts: Vec<FontVec>,
    /// Parallel to `fonts`: the face's `trak` table, when it has one.
    tracking_tables: Vec<Option<TrackingTable>>,
    pixel_size: f32,
    /// The nominal point size (drives the `trak` lookup); equals `pixel_size` / scale factor.
    point_size: f32,
    cache: HashMap<GlyphCacheKey, CachedGlyph>,
}

impl TextRasterizer {
    /// Loads the platform font chain for `style`. Never fails: an empty chain draws
    /// nothing and measures every glyph as `0.6 × size` so layout stays sane.
    pub fn load_system(style: FontStyle) -> TextRasterizer {
        let mut fonts = Vec::new();
        let mut tracking_tables = Vec::new();
        for file in platform_font_files(style) {
            match Self::load_font_file(&file, style.weight) {
                Some((font, tracking)) => {
                    fonts.push(font);
                    tracking_tables.push(tracking);
                }
                None => super::log(&format!("text: could not load font {}", file.path.display())),
            }
        }
        if fonts.is_empty() {
            super::log("text: no system font could be loaded; text will not render");
        }
        TextRasterizer { fonts, tracking_tables, pixel_size: 12.0, point_size: 12.0, cache: HashMap::new() }
    }

    /// A rasterizer over caller-supplied font bytes (tests, embedded fonts).
    #[cfg(test)]
    pub fn from_font_data(font_files: Vec<Vec<u8>>, weight: f32) -> TextRasterizer {
        let mut fonts = Vec::new();
        let mut tracking_tables = Vec::new();
        for data in font_files {
            let tracking = TrackingTable::parse(&data);
            if let Ok(mut font) = FontVec::try_from_vec(data) {
                Self::apply_ui_text_variations(&mut font, weight);
                fonts.push(font);
                tracking_tables.push(tracking);
            }
        }
        TextRasterizer { fonts, tracking_tables, pixel_size: 12.0, point_size: 12.0, cache: HashMap::new() }
    }

    fn load_font_file(file: &FontFile, weight: f32) -> Option<(FontVec, Option<TrackingTable>)> {
        let data = std::fs::read(&file.path).ok()?;
        // Collections (`.ttc`) share one table directory per face; only single faces get tracking.
        let tracking = if file.collection_index == 0 && !data.starts_with(b"ttcf") {
            TrackingTable::parse(&data)
        } else {
            None
        };
        let mut font = FontVec::try_from_vec_and_index(data, file.collection_index).ok()?;
        Self::apply_ui_text_variations(&mut font, weight);
        Some((font, tracking))
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

    /// The em size in pixels used by all following measure/draw calls; the point size is
    /// taken to be the same (scale factor 1).
    #[cfg(test)]
    pub fn set_pixel_size(&mut self, pixel_size: f32) {
        self.set_size(pixel_size, 1.0);
    }

    /// Text at `point_size` on a display with `scale_factor` physical pixels per point
    /// (em size = point size × scale factor; SF's `trak` tracking follows the point size).
    pub fn set_size(&mut self, point_size: f32, scale_factor: f32) {
        self.point_size = point_size.max(1.0);
        self.pixel_size = (point_size * scale_factor).max(1.0);
    }

    /// Extra advance per glyph in pixels from the face's `trak` table at the current size.
    fn tracking_pixels(&self, font_index: usize) -> f32 {
        let Some(Some(table)) = self.tracking_tables.get(font_index) else { return 0.0 };
        let units_per_em = self.fonts[font_index].units_per_em().unwrap_or(1000.0);
        table.tracking_units(self.point_size) / units_per_em * self.pixel_size
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
            pen_x += glyph.advance + self.tracking_pixels(font_index);
            previous = Some((font_index, glyph_id));
        }
        pen_x - x
    }

    /// Like `rasterize_line`, but every ASCII digit takes the width of "0" with the glyph
    /// centred in that slot — SwiftUI's `.monospacedDigit()` (tabular figures) without
    /// OpenType feature support. Returns the total advance.
    pub fn rasterize_line_tabular_digits(&mut self, text: &str, x: f32, y: f32, mut sink: impl FnMut(i32, i32, f32)) -> f32 {
        let digit_slot_width = self.measure("0");
        let mut pen_x = x;
        for character in text.chars() {
            let mut buffer = [0u8; 4];
            let piece: &str = character.encode_utf8(&mut buffer);
            if character.is_ascii_digit() {
                let glyph_width = self.measure(piece);
                let inset = ((digit_slot_width - glyph_width) / 2.0).max(0.0);
                self.rasterize_line(piece, pen_x + inset, y, &mut sink);
                pen_x += digit_slot_width;
            } else {
                pen_x += self.rasterize_line(piece, pen_x, y, &mut sink);
            }
        }
        pen_x - x
    }

    /// Advance width of `text` laid out with tabular digits (`rasterize_line_tabular_digits`).
    pub fn measure_tabular_digits(&mut self, text: &str) -> f32 {
        self.rasterize_line_tabular_digits(text, 0.0, 0.0, |_, _, _| {})
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

    #[test]
    fn tracking_table_parses_a_synthetic_trak_and_interpolates() {
        // A minimal sfnt with one table: `trak` (horizontal, one default track, three sizes).
        let mut trak: Vec<u8> = Vec::new();
        trak.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // version
        trak.extend_from_slice(&0u16.to_be_bytes()); // format
        trak.extend_from_slice(&12u16.to_be_bytes()); // horizOffset
        trak.extend_from_slice(&0u16.to_be_bytes()); // vertOffset
        trak.extend_from_slice(&0u16.to_be_bytes()); // reserved
        // TrackData at 12: nTracks 1, nSizes 3, sizeTableOffset (from table start).
        let track_data_start = 12;
        let size_table_offset = track_data_start + 8 + 8; // after one track entry
        let values_offset = size_table_offset + 4 * 3;
        trak.extend_from_slice(&1u16.to_be_bytes());
        trak.extend_from_slice(&3u16.to_be_bytes());
        trak.extend_from_slice(&(size_table_offset as u32).to_be_bytes());
        // Track entry: track 0.0 (16.16), nameIndex, offset to values.
        trak.extend_from_slice(&0i32.to_be_bytes());
        trak.extend_from_slice(&0u16.to_be_bytes());
        trak.extend_from_slice(&(values_offset as u16).to_be_bytes());
        // Sizes 6, 12, 24 pt (16.16).
        for size in [6.0f32, 12.0, 24.0] {
            trak.extend_from_slice(&((size * 65536.0) as i32).to_be_bytes());
        }
        // Values 100, 0, -20 units.
        for value in [100i16, 0, -20] {
            trak.extend_from_slice(&value.to_be_bytes());
        }
        let mut font_data: Vec<u8> = Vec::new();
        font_data.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // sfnt version
        font_data.extend_from_slice(&1u16.to_be_bytes()); // numTables
        font_data.extend_from_slice(&[0u8; 6]); // searchRange etc.
        let table_offset = 12 + 16;
        font_data.extend_from_slice(b"trak");
        font_data.extend_from_slice(&0u32.to_be_bytes()); // checksum
        font_data.extend_from_slice(&(table_offset as u32).to_be_bytes());
        font_data.extend_from_slice(&(trak.len() as u32).to_be_bytes());
        font_data.extend_from_slice(&trak);

        let table = TrackingTable::parse(&font_data).expect("trak parses");
        assert_eq!(table.sizes, vec![6.0, 12.0, 24.0]);
        assert_eq!(table.values, vec![100.0, 0.0, -20.0]);
        assert_eq!(table.tracking_units(3.0), 100.0, "clamped below");
        assert_eq!(table.tracking_units(9.0), 50.0, "interpolated");
        assert_eq!(table.tracking_units(18.0), -10.0);
        assert_eq!(table.tracking_units(40.0), -20.0, "clamped above");
        // Fonts without the table, or junk, give None.
        assert_eq!(TrackingTable::parse(b"not a font"), None);
        assert_eq!(TrackingTable::parse(&font_data[..20]), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn system_rounded_font_applies_sf_tracking_at_small_sizes() {
        // SF Rounded's trak table widens 9.5 pt text noticeably (~0.5 pt per glyph);
        // at 28 pt the tracking is near zero.
        let mut rasterizer = TextRasterizer::load_system(FontStyle::LABEL);
        assert!(rasterizer.has_fonts());
        let table = rasterizer.tracking_tables[0].as_ref().expect("SF Rounded has a trak table");
        assert!(table.tracking_units(9.5) > 80.0, "units at 9.5 pt: {}", table.tracking_units(9.5));
        rasterizer.set_size(9.5, 2.0);
        let tracked = rasterizer.measure("pass_finder");
        let per_glyph = rasterizer.tracking_pixels(0);
        assert!(per_glyph > 0.8 && per_glyph < 1.6, "tracking px per glyph at 9.5 pt @2x: {per_glyph}");
        assert!(tracked > 100.0, "tracked width {tracked}");
        // Tabular digits: "11%" and "00%" take the same width.
        assert!((rasterizer.measure_tabular_digits("11%") - rasterizer.measure_tabular_digits("00%")).abs() < 0.01);
        assert!(rasterizer.measure("11%") < rasterizer.measure_tabular_digits("11%"));
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
