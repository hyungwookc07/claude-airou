//! Pet definitions: JSON "sprite packs" (palette + pixel-art frames per state).
//! Port of `Pets/PetDefinition.swift` and `Pets/PetLibrary.swift`. The JSON format is shared
//! with the Swift app — the built-ins are included straight from the Swift resources so
//! there is exactly one source of truth in the repo.

use crate::model::PetState;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const TRANSPARENT_CHARS: [char; 2] = ['.', ' '];
pub const DEFAULT_FPS: f64 = 3.0;
pub const MIN_GRID_SIDE: usize = 4;
pub const MAX_GRID_SIDE: usize = 64;

/// The eight built-in pets, embedded from the Swift resource directory at compile time.
/// Order here is the menu order (Airou first, like `BuiltInPets.embeddedJSONFiles`).
pub const BUILT_IN_PET_SOURCES: [(&str, &str); 8] = [
    ("airou-felyne.json", include_str!("../../Sources/ClaudeAirou/Resources/pets/airou-felyne.json")),
    ("mochi-cat.json", include_str!("../../Sources/ClaudeAirou/Resources/pets/mochi-cat.json")),
    ("quackers-duck.json", include_str!("../../Sources/ClaudeAirou/Resources/pets/quackers-duck.json")),
    ("boo-ghost.json", include_str!("../../Sources/ClaudeAirou/Resources/pets/boo-ghost.json")),
    ("jelly-slime.json", include_str!("../../Sources/ClaudeAirou/Resources/pets/jelly-slime.json")),
    ("bolt-robot.json", include_str!("../../Sources/ClaudeAirou/Resources/pets/bolt-robot.json")),
    ("inky-octopus.json", include_str!("../../Sources/ClaudeAirou/Resources/pets/inky-octopus.json")),
    ("clawd-claude.json", include_str!("../../Sources/ClaudeAirou/Resources/pets/clawd-claude.json")),
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PetDefinition {
    pub id: String,
    pub name: String,
    pub species: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub fps: Option<f64>,
    pub palette: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub phrases: Option<BTreeMap<String, Vec<String>>>,
    pub frames: BTreeMap<String, Vec<Vec<String>>>,
}

#[derive(Debug)]
pub struct ValidationError {
    pub problems: Vec<String>,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.problems.join("\n"))
    }
}
impl std::error::Error for ValidationError {}

impl PetDefinition {
    pub fn decode(data: &[u8]) -> Result<PetDefinition, serde_json::Error> {
        serde_json::from_slice(data)
    }

    pub fn load(path: &std::path::Path) -> Result<PetDefinition, String> {
        let _ = path;
        todo!("read file + decode, map errors to readable strings")
    }

    /// `fps` clamped to 0.5–12, default 3 (mirrors `framesPerSecond`).
    pub fn frames_per_second(&self) -> f64 {
        todo!()
    }

    /// Random click phrases; falls back to ["♥"] when none defined.
    pub fn pet_phrases(&self) -> Vec<String> {
        todo!()
    }

    /// Grid size derived from the idle frames: (width, height).
    pub fn grid_size(&self) -> (usize, usize) {
        todo!()
    }

    /// Frames for a state, following the fallback chain (`PetState::fallback_states`)
    /// and finally `idle`.
    pub fn frames_for(&self, state: PetState) -> &[Vec<String>] {
        let _ = state;
        todo!()
    }

    /// Port of `PetDefinition.validate()`: returns non-fatal warnings, `Err` for problems
    /// that make the pet unusable. Every rule and message shape from the Swift version:
    /// id charset, non-empty name, single-char palette keys, reserved transparency chars,
    /// #RRGGBB/#RRGGBBAA colors, frames.idle required, uniform grid (min 4, max 64),
    /// every used char in palette, unknown state keys warned, unused palette keys warned,
    /// missing states warned with their fallback, duplicate problems deduped in order.
    pub fn validate(&self) -> Result<Vec<String>, ValidationError> {
        todo!("port PetDefinition.validate from Sources/ClaudeAirou/Pets/PetDefinition.swift")
    }
}

/// A palette color parsed from `#RRGGBB` or `#RRGGBBAA`; components 0–255 plus alpha.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PixelColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl PixelColor {
    /// Rejects signs and wrong lengths, exactly like Swift's `PixelColor(hex:)`.
    pub fn parse(hex: &str) -> Option<PixelColor> {
        let _ = hex;
        todo!()
    }
}

/// Palette resolved to characters, ready for rendering (invalid entries skipped).
pub struct ResolvedPalette {
    pub colors: std::collections::HashMap<char, PixelColor>,
}

impl ResolvedPalette {
    pub fn new(definition: &PetDefinition) -> ResolvedPalette {
        let _ = definition;
        todo!()
    }
}

pub struct LoadedPet {
    pub definition: PetDefinition,
    /// None for built-ins.
    pub source_path: Option<PathBuf>,
}

impl LoadedPet {
    pub fn is_built_in(&self) -> bool {
        self.source_path.is_none()
    }
}

pub struct PetLibrary {
    pub pets: Vec<LoadedPet>,
    pub load_problems: Vec<String>,
}

impl PetLibrary {
    /// Built-ins first (menu order), then `~/.claude-airou/pets/*.json` sorted by filename;
    /// a user pet with the same id as a built-in overrides it in place. Unreadable user
    /// files land in `load_problems` ("<file>: <why>"), invalid built-ins go to stderr.
    pub fn load() -> PetLibrary {
        Self::load_from(&crate::paths::pets_dir())
    }

    pub fn load_from(user_pets_dir: &std::path::Path) -> PetLibrary {
        let _ = user_pets_dir;
        todo!("port PetLibrary.load from Sources/ClaudeAirou/Pets/PetLibrary.swift")
    }

    pub fn pet_with_id(&self, id: &str) -> Option<&LoadedPet> {
        self.pets.iter().find(|pet| pet.definition.id == id)
    }

    /// The pet to show: the configured one if it still exists, else the first built-in.
    pub fn resolve_selected(&self, preferred_id: Option<&str>) -> Option<&LoadedPet> {
        preferred_id
            .and_then(|id| self.pet_with_id(id))
            .or_else(|| self.pets.first())
    }
}
