//! Renders pet frames to PNG (`claude-airou render`) and ASCII (`claude-airou preview`).
//! Port of `Pets/SpriteRenderer.swift` using the `png` crate instead of ImageIO.
//! The live overlay draws into its own framebuffer instead (see overlay/).

use crate::model::PetState;
use crate::pets::{PetDefinition, PixelColor, ResolvedPalette};
use std::path::{Path, PathBuf};

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
    let _ = (pet, output_dir, pixel_scale, background);
    todo!("port SpriteRenderer.renderAll from Sources/ClaudeAirou/Pets/SpriteRenderer.swift")
}

/// Renders one frame to raw RGBA8 (width*height*4, row-major, top-down). Shared by
/// `render_all` and the overlay's software renderer.
pub fn frame_rgba(
    frame: &[String],
    palette: &ResolvedPalette,
    pixel_scale: u32,
    background: Option<PixelColor>,
) -> Result<(Vec<u8>, u32, u32), String> {
    let _ = (frame, palette, pixel_scale, background);
    todo!()
}

/// ASCII preview: transparent → space, everything else → the palette character
/// (or `#` when `solid`).
pub fn ascii_art(frame: &[String], solid: bool) -> String {
    let _ = (frame, solid);
    todo!("port SpriteRenderer.asciiArt")
}

/// Convenience for `preview_pet`/`hatch_pet` MCP tools and CLI render: render the whole
/// contact sheet into a temp dir and return the PNG bytes of sheet.png.
pub fn sheet_png_bytes(pet: &PetDefinition, pixel_scale: u32) -> Result<Vec<u8>, String> {
    let _ = (pet, pixel_scale);
    todo!()
}

#[allow(dead_code)]
pub fn all_states() -> [PetState; 8] {
    PetState::ALL
}
