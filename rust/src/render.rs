//! Renders pet frames to PNG (`claude-airou render`) and ASCII (`claude-airou preview`).
//! Port of `Pets/SpriteRenderer.swift` using the `png` crate instead of ImageIO.
//! The live overlay draws into its own framebuffer instead (see overlay/).

use crate::model::PetState;
use crate::pets::{PetDefinition, PixelColor, ResolvedPalette, TRANSPARENT_CHARS};
use std::path::{Path, PathBuf};

/// Sheet background when no explicit background is given (Swift: `#3a3f4b`, alpha forced to 1).
const DEFAULT_SHEET_BACKGROUND: PixelColor = PixelColor { r: 0x3a, g: 0x3f, b: 0x4b, a: 0xff };

/// Writes `<state>_<index>.png` for every frame plus `sheet.png` (rows = states in
/// `PetState::ALL` order, columns = frames, gutter = 2×scale, default sheet background
/// #3a3f4b when `background` is None). Returns the written file paths.
/// Pixel mapping identical to Swift: sprite pixel → scale×scale block, straight alpha,
/// transparent characters skipped (sheet keeps its background).
pub fn render_all(
    pet: &PetDefinition,
    output_dir: &Path,
    pixel_scale: u32,
    background: Option<PixelColor>,
) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(output_dir)
        .map_err(|error| format!("could not create directory at {}: {error}", output_dir.display()))?;
    let palette = ResolvedPalette::new(pet);
    let mut written: Vec<PathBuf> = Vec::new();

    // Sheet geometry first: validation caps the grid at 64x64 but not the frame count, so
    // size the sheet in u64 and refuse absurd ones with a clean error instead of aborting
    // on allocation (rust-only guard; Swift fails via a thrown CGContext error, but only
    // after writing the per-frame PNGs — failing before any file is written is deliberate).
    let (grid_width, grid_height) = pet.grid_size();
    let cell_width = grid_width as u32 * pixel_scale;
    let cell_height = grid_height as u32 * pixel_scale;
    let gutter = pixel_scale * 2;
    let max_frame_count = PetState::ALL
        .iter()
        .map(|state| pet.frames_for(*state).len())
        .max()
        .unwrap_or(0);
    const MAX_SHEET_BYTES: u64 = 512 * 1024 * 1024;
    let sheet_width_u64 = max_frame_count as u64 * (cell_width as u64 + gutter as u64) + gutter as u64;
    let sheet_height_u64 = PetState::ALL.len() as u64 * (cell_height as u64 + gutter as u64) + gutter as u64;
    if sheet_width_u64.saturating_mul(sheet_height_u64).saturating_mul(4) > MAX_SHEET_BYTES {
        return Err(format!(
            "could not create sheet context: {sheet_width_u64}x{sheet_height_u64} pixels is too large — reduce --scale or the number of frames"
        ));
    }
    let sheet_width = sheet_width_u64 as u32;
    let sheet_height = sheet_height_u64 as u32;

    for state in PetState::ALL {
        for (index, frame) in pet.frames_for(state).iter().enumerate() {
            let (rgba, width, height) = frame_rgba(frame, &palette, pixel_scale, background)?;
            let path = output_dir.join(format!("{}_{index}.png", state.raw()));
            write_png(&path, width, height, &rgba)?;
            written.push(path);
        }
    }

    // Contact sheet: rows = states (in PetState order), columns = frames.

    // Swift fills the sheet with alpha forced to 1 regardless of the background's alpha.
    let base = background.unwrap_or(DEFAULT_SHEET_BACKGROUND);
    let sheet_background = PixelColor { r: base.r, g: base.g, b: base.b, a: 0xff };
    let mut sheet = vec![0u8; sheet_width as usize * sheet_height as usize * 4];
    for pixel in sheet.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[sheet_background.r, sheet_background.g, sheet_background.b, sheet_background.a]);
    }

    for (state_index, state) in PetState::ALL.iter().enumerate() {
        let frames = pet.frames_for(*state);
        for (frame_index, frame) in frames.iter().enumerate() {
            let (rgba, width, height) = frame_rgba(frame, &palette, pixel_scale, None)?;
            let x = gutter + frame_index as u32 * (cell_width + gutter);
            let y = gutter + state_index as u32 * (cell_height + gutter);
            composite_image(&mut sheet, sheet_width, sheet_height, &rgba, width, height, x, y);
        }
    }
    let sheet_path = output_dir.join("sheet.png");
    write_png(&sheet_path, sheet_width, sheet_height, &sheet)?;
    written.push(sheet_path);
    Ok(written)
}

/// Renders one frame to raw RGBA8 (width*height*4, row-major, top-down). Shared by
/// `render_all` and the overlay's software renderer.
pub fn frame_rgba(
    frame: &[String],
    palette: &ResolvedPalette,
    pixel_scale: u32,
    background: Option<PixelColor>,
) -> Result<(Vec<u8>, u32, u32), String> {
    let grid_height = frame.len();
    let grid_width = frame.first().map(|row| row.chars().count()).unwrap_or(0);
    if grid_width == 0 || grid_height == 0 {
        return Err("frame is empty".to_string());
    }
    // 64x64 grid at scale 64 is exactly 64 MB of RGBA — anything beyond that means an
    // unvalidated definition; fail cleanly rather than overflow or abort on allocation.
    const MAX_FRAME_BYTES: u64 = 64 * 1024 * 1024;
    let width_u64 = grid_width as u64 * pixel_scale as u64;
    let height_u64 = grid_height as u64 * pixel_scale as u64;
    if width_u64.saturating_mul(height_u64).saturating_mul(4) > MAX_FRAME_BYTES {
        return Err(format!("frame too large to render ({width_u64}x{height_u64} pixels)"));
    }
    let width = width_u64 as u32;
    let height = height_u64 as u32;
    let mut buffer = vec![0u8; width as usize * height as usize * 4];
    if let Some(color) = background {
        for pixel in buffer.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[color.r, color.g, color.b, color.a]);
        }
    }
    for (row_index, row) in frame.iter().enumerate() {
        for (column_index, character) in row.chars().enumerate() {
            let Some(color) = palette.colors.get(&character) else { continue };
            fill_block(
                &mut buffer,
                width,
                height,
                column_index as u32 * pixel_scale,
                row_index as u32 * pixel_scale,
                pixel_scale,
                *color,
            );
        }
    }
    Ok((buffer, width, height))
}

/// ASCII preview: transparent → space, everything else → the palette character
/// (or `#` when `solid`).
pub fn ascii_art(frame: &[String], solid: bool) -> String {
    frame
        .iter()
        .map(|row| {
            row.chars()
                .map(|character| {
                    if TRANSPARENT_CHARS.contains(&character) {
                        ' '
                    } else if solid {
                        '#'
                    } else {
                        character
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<String>>()
        .join("\n")
}

/// Convenience for `preview_pet`/`hatch_pet` MCP tools and CLI render: render the whole
/// contact sheet into a temp dir and return the PNG bytes of sheet.png.
pub fn sheet_png_bytes(pet: &PetDefinition, pixel_scale: u32) -> Result<Vec<u8>, String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "claude-airou-render-{}-{}-{}",
        std::process::id(),
        nanos,
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = render_all(pet, &dir, pixel_scale, None).and_then(|_| {
        let sheet_path = dir.join("sheet.png");
        std::fs::read(&sheet_path)
            .map_err(|error| format!("could not read PNG at {}: {error}", sheet_path.display()))
    });
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[allow(dead_code)]
pub fn all_states() -> [PetState; 8] {
    PetState::ALL
}

/// Fills a `scale`×`scale` block at (x, y), compositing the straight-alpha color over
/// whatever is already in the buffer (like CoreGraphics' default source-over fill).
/// Pixels outside the buffer are clipped, matching CGContext.fill's clipping.
fn fill_block(buffer: &mut [u8], width: u32, height: u32, x: u32, y: u32, scale: u32, color: PixelColor) {
    if color.a == 0 {
        return;
    }
    let x_end = x.saturating_add(scale).min(width);
    let y_end = y.saturating_add(scale).min(height);
    for py in y.min(height)..y_end {
        for px in x.min(width)..x_end {
            let offset = (py as usize * width as usize + px as usize) * 4;
            composite_over(&mut buffer[offset..offset + 4], color);
        }
    }
}

/// Straight-alpha source-over: `out = src over dst`.
fn composite_over(dst: &mut [u8], src: PixelColor) {
    if src.a == 255 {
        dst.copy_from_slice(&[src.r, src.g, src.b, src.a]);
        return;
    }
    let source_alpha = src.a as f64 / 255.0;
    let dest_alpha = dst[3] as f64 / 255.0;
    let out_alpha = source_alpha + dest_alpha * (1.0 - source_alpha);
    if out_alpha <= 0.0 {
        dst.copy_from_slice(&[0, 0, 0, 0]);
        return;
    }
    let blend = |source: u8, dest: u8| -> u8 {
        let source = source as f64 / 255.0;
        let dest = dest as f64 / 255.0;
        let out = (source * source_alpha + dest * dest_alpha * (1.0 - source_alpha)) / out_alpha;
        (out * 255.0).round().clamp(0.0, 255.0) as u8
    };
    let r = blend(src.r, dst[0]);
    let g = blend(src.g, dst[1]);
    let b = blend(src.b, dst[2]);
    dst[0] = r;
    dst[1] = g;
    dst[2] = b;
    dst[3] = (out_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
}

/// Composites a straight-alpha RGBA image over the sheet at (x, y), clipped to the sheet.
fn composite_image(
    sheet: &mut [u8],
    sheet_width: u32,
    sheet_height: u32,
    image: &[u8],
    image_width: u32,
    image_height: u32,
    x: u32,
    y: u32,
) {
    for row in 0..image_height {
        let dest_y = y + row;
        if dest_y >= sheet_height {
            break;
        }
        for column in 0..image_width {
            let dest_x = x + column;
            if dest_x >= sheet_width {
                break;
            }
            let source_offset = (row as usize * image_width as usize + column as usize) * 4;
            let source = PixelColor {
                r: image[source_offset],
                g: image[source_offset + 1],
                b: image[source_offset + 2],
                a: image[source_offset + 3],
            };
            if source.a == 0 {
                continue;
            }
            let dest_offset = (dest_y as usize * sheet_width as usize + dest_x as usize) * 4;
            composite_over(&mut sheet[dest_offset..dest_offset + 4], source);
        }
    }
}

fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let write_error = |error: &dyn std::fmt::Display| format!("could not write PNG at {}: {error}", path.display());
    let file = std::fs::File::create(path)
        .map_err(|error| format!("could not create PNG destination at {}: {error}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|error| write_error(&error))?;
    writer.write_image_data(rgba).map_err(|error| write_error(&error))?;
    writer.finish().map_err(|error| write_error(&error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn rows(rows: &[&str]) -> Vec<String> {
        rows.iter().map(|row| row.to_string()).collect()
    }

    /// A valid 4x4 pet with 2 idle frames and 1 thinking frame; 'g' is semi-transparent.
    fn tiny_pet() -> PetDefinition {
        let mut palette = BTreeMap::new();
        palette.insert("r".to_string(), "#FF0000".to_string());
        palette.insert("g".to_string(), "#00FF0080".to_string());
        let mut frames = BTreeMap::new();
        frames.insert(
            "idle".to_string(),
            vec![
                rows(&["rrrr", "r..r", "r..r", "rrrr"]),
                rows(&["....", ".rr.", ".rr.", "...."]),
            ],
        );
        frames.insert("thinking".to_string(), vec![rows(&["gggg", "g..g", "g..g", "gggg"])]);
        PetDefinition {
            id: "tiny".to_string(),
            name: "Tiny".to_string(),
            species: "test".to_string(),
            description: None,
            author: None,
            fps: None,
            palette,
            phrases: None,
            frames,
        }
    }

    fn decode_png(path: &Path) -> (Vec<u8>, u32, u32) {
        let decoder = png::Decoder::new(std::fs::File::open(path).unwrap());
        let mut reader = decoder.read_info().unwrap();
        let mut buffer = vec![0u8; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buffer).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!(info.bit_depth, png::BitDepth::Eight);
        buffer.truncate(info.buffer_size());
        (buffer, info.width, info.height)
    }

    fn pixel(buffer: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let offset = (y as usize * width as usize + x as usize) * 4;
        [buffer[offset], buffer[offset + 1], buffer[offset + 2], buffer[offset + 3]]
    }

    #[test]
    fn tiny_pet_is_valid() {
        tiny_pet().validate().expect("tiny pet must validate (warnings only)");
    }

    #[test]
    fn oversized_sheet_fails_cleanly_before_writing_anything() {
        // 4x4 grid, scale 64, 3000 idle frames → sheet ≈ 1.15M x 3.2K px ≈ 59 GB of RGBA.
        let mut pet = tiny_pet();
        let frame = pet.frames.get("idle").unwrap()[0].clone();
        pet.frames.insert("idle".to_string(), vec![frame; 3000]);
        let dir = tempfile::tempdir().unwrap();
        let error = render_all(&pet, dir.path(), 64, None).expect_err("must refuse");
        assert!(error.contains("could not create sheet context"), "got: {error}");
        let leftovers = std::fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(leftovers, 0, "no per-frame PNGs before the guard");
    }

    #[test]
    fn oversized_frame_fails_cleanly() {
        // An unvalidated 100k-wide row must not overflow u32 or abort on allocation.
        let palette = ResolvedPalette::new(&tiny_pet());
        let frame = vec!["r".repeat(100_000); 4];
        let error = frame_rgba(&frame, &palette, 64, None).expect_err("must refuse");
        assert!(error.contains("frame too large"), "got: {error}");
    }

    #[test]
    fn ascii_art_exact() {
        let frame = rows(&["..ab", " a.b"]);
        assert_eq!(ascii_art(&frame, false), "  ab\n a b");
        assert_eq!(ascii_art(&frame, true), "  ##\n # #");
        assert_eq!(ascii_art(&[], false), "");
    }

    #[test]
    fn frame_rgba_maps_pixels_to_scaled_blocks() {
        let pet = tiny_pet();
        let palette = ResolvedPalette::new(&pet);
        let frame = rows(&["r.", ".r"]);
        let (buffer, width, height) = frame_rgba(&frame, &palette, 2, None).unwrap();
        assert_eq!((width, height), (4, 4));
        // Top-left 2x2 block is opaque red; its right neighbour transparent.
        assert_eq!(pixel(&buffer, width, 0, 0), [255, 0, 0, 255]);
        assert_eq!(pixel(&buffer, width, 1, 1), [255, 0, 0, 255]);
        assert_eq!(pixel(&buffer, width, 2, 0), [0, 0, 0, 0]);
        // Bottom-right block red again (top-down row order).
        assert_eq!(pixel(&buffer, width, 3, 3), [255, 0, 0, 255]);
        assert_eq!(pixel(&buffer, width, 0, 3), [0, 0, 0, 0]);
    }

    #[test]
    fn frame_rgba_fills_background_and_composites_alpha() {
        let pet = tiny_pet();
        let palette = ResolvedPalette::new(&pet);
        let frame = rows(&["g."]);
        let background = PixelColor { r: 255, g: 255, b: 255, a: 255 };
        let (buffer, width, _) = frame_rgba(&frame, &palette, 1, Some(background)).unwrap();
        // Transparent sprite pixel keeps the background.
        assert_eq!(pixel(&buffer, width, 1, 0), [255, 255, 255, 255]);
        // Semi-transparent green (#00FF0080) over white: still opaque, greener than red/blue.
        let blended = pixel(&buffer, width, 0, 0);
        assert_eq!(blended[3], 255);
        assert_eq!(blended[0], blended[2]);
        assert!(blended[1] > blended[0]);
        // Straight alpha-over with sa = 128/255: r = 255*(1-sa) ≈ 127, g = 255*sa + 255*(1-sa) = 255.
        assert_eq!(blended[0], 127);
        assert_eq!(blended[1], 255);
    }

    #[test]
    fn frame_rgba_rejects_empty_frames() {
        let pet = tiny_pet();
        let palette = ResolvedPalette::new(&pet);
        assert_eq!(frame_rgba(&[], &palette, 2, None).unwrap_err(), "frame is empty");
        assert_eq!(
            frame_rgba(&[String::new()], &palette, 2, None).unwrap_err(),
            "frame is empty"
        );
    }

    #[test]
    fn render_all_writes_expected_file_set_and_sheet() {
        let pet = tiny_pet();
        let dir = tempfile::tempdir().unwrap();
        let written = render_all(&pet, dir.path(), 3, None).unwrap();

        // Fallbacks resolve to: idle-backed states get 2 frames, thinking-backed get 1.
        let mut names: Vec<String> = written
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        names.sort();
        let mut expected = vec![
            "hello_0.png", "hello_1.png",                       // -> done -> idle (2 frames)
            "idle_0.png", "idle_1.png",
            "thinking_0.png",
            "working_0.png",                                     // -> thinking (1 frame)
            "waiting_approval_0.png", "waiting_approval_1.png",  // -> needs_input -> idle
            "needs_input_0.png", "needs_input_1.png",            // -> waiting_approval -> idle
            "done_0.png", "done_1.png",                          // -> idle
            "error_0.png", "error_1.png",                        // -> idle
            "sheet.png",
        ];
        expected.sort();
        assert_eq!(names, expected);
        for path in &written {
            assert!(path.exists(), "missing {}", path.display());
        }

        // Per-frame PNG: 4x4 grid at scale 3 = 12x12, no background.
        let (idle0, width, height) = decode_png(&dir.path().join("idle_0.png"));
        assert_eq!((width, height), (12, 12));
        assert_eq!(pixel(&idle0, width, 0, 0), [255, 0, 0, 255]); // 'r'
        assert_eq!(pixel(&idle0, width, 4, 4), [0, 0, 0, 0]); // interior '.'

        // Sheet: maxFrames=2, cell=12, gutter=6 -> 2*(12+6)+6 = 42 wide, 8*(12+6)+6 = 150 tall.
        let (sheet, sheet_width, sheet_height) = decode_png(&dir.path().join("sheet.png"));
        assert_eq!((sheet_width, sheet_height), (42, 150));
        // Gutter pixel: default background #3a3f4b, fully opaque.
        assert_eq!(pixel(&sheet, sheet_width, 0, 0), [0x3a, 0x3f, 0x4b, 0xff]);
        // idle is row index 1 (PetState::ALL order), frame 0 cell starts at (6, 24).
        assert_eq!(pixel(&sheet, sheet_width, 6, 24), [255, 0, 0, 255]);
        // Its transparent interior shows the sheet background, not transparency.
        assert_eq!(pixel(&sheet, sheet_width, 6 + 4, 24 + 4), [0x3a, 0x3f, 0x4b, 0xff]);
        // thinking is row index 2 at (6, 42): semi-transparent green over the background.
        let blended = pixel(&sheet, sheet_width, 6, 42);
        assert_eq!(blended[3], 255);
        assert!(blended[1] > blended[0] && blended[1] > blended[2]);
        // hello (row 0) falls back to idle frames, so frame 1 exists at column 1.
        // idle frame 1 top row is transparent -> background; its (1,1) pixel is 'r'.
        let hello_cell_x = 6 + 18;
        assert_eq!(pixel(&sheet, sheet_width, hello_cell_x, 6), [0x3a, 0x3f, 0x4b, 0xff]);
        assert_eq!(pixel(&sheet, sheet_width, hello_cell_x + 3, 6 + 3), [255, 0, 0, 255]);
    }

    #[test]
    fn render_all_with_explicit_background_keeps_it_opaque_on_sheet() {
        let pet = tiny_pet();
        let dir = tempfile::tempdir().unwrap();
        let background = PixelColor::parse("#12345678").unwrap();
        render_all(&pet, dir.path(), 1, Some(background)).unwrap();
        // Per-frame files keep the background's alpha…
        let (idle0, width, _) = decode_png(&dir.path().join("idle_0.png"));
        assert_eq!(pixel(&idle0, width, 1, 1), [0x12, 0x34, 0x56, 0x78]);
        // …but the sheet forces alpha to 1 like Swift does.
        let (sheet, sheet_width, _) = decode_png(&dir.path().join("sheet.png"));
        assert_eq!(pixel(&sheet, sheet_width, 0, 0), [0x12, 0x34, 0x56, 0xff]);
    }

    #[test]
    fn sheet_png_bytes_returns_png_magic() {
        let pet = tiny_pet();
        let bytes = sheet_png_bytes(&pet, 2).unwrap();
        assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']));
    }

    #[test]
    fn render_all_renders_every_built_in() {
        // Smoke test: all 8 built-ins render without error at scale 1.
        for (file_name, source) in crate::pets::BUILT_IN_PET_SOURCES {
            let pet = PetDefinition::decode(source.as_bytes()).unwrap();
            let dir = tempfile::tempdir().unwrap();
            let written = render_all(&pet, dir.path(), 1, None)
                .unwrap_or_else(|error| panic!("{file_name} failed to render: {error}"));
            assert!(written.iter().any(|path| path.ends_with("sheet.png")), "{file_name}");
        }
    }
}
