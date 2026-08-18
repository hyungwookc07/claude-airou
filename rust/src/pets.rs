//! Pet definitions: JSON "sprite packs" (palette + pixel-art frames per state).
//! Port of `Pets/PetDefinition.swift` and `Pets/PetLibrary.swift`. The JSON format is shared
//! with the Swift app — the built-ins are included straight from the Swift resources so
//! there is exactly one source of truth in the repo.

use crate::model::PetState;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
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
        let data = std::fs::read(path).map_err(|error| format!("could not read file: {error}"))?;
        Self::decode(&data).map_err(|error| format!("invalid pet JSON: {error}"))
    }

    /// `fps` clamped to 0.5–12, default 3 (mirrors `framesPerSecond`).
    pub fn frames_per_second(&self) -> f64 {
        match self.fps {
            Some(fps) if fps > 0.0 => fps.max(0.5).min(12.0),
            _ => DEFAULT_FPS,
        }
    }

    /// Random click phrases; falls back to ["♥"] when none defined.
    pub fn pet_phrases(&self) -> Vec<String> {
        let list = self
            .phrases
            .as_ref()
            .and_then(|phrases| phrases.get("pet"))
            .cloned()
            .unwrap_or_default();
        if list.is_empty() {
            vec!["♥".to_string()]
        } else {
            list
        }
    }

    /// Grid size derived from the idle frames: (width, height).
    pub fn grid_size(&self) -> (usize, usize) {
        let frames = self.frames_for(PetState::Idle);
        match frames.first().and_then(|frame| frame.first().map(|row| (row, frame))) {
            Some((first_row, first_frame)) => (first_row.chars().count(), first_frame.len()),
            None => (0, 0),
        }
    }

    /// Frames for a state, following the fallback chain (`PetState::fallback_states`)
    /// and finally `idle`.
    pub fn frames_for(&self, state: PetState) -> &[Vec<String>] {
        if let Some(direct) = self.frames.get(state.raw()) {
            if !direct.is_empty() {
                return direct;
            }
        }
        for fallback in state.fallback_states() {
            if let Some(candidate) = self.frames.get(fallback.raw()) {
                if !candidate.is_empty() {
                    return candidate;
                }
            }
        }
        self.frames.get("idle").map(|frames| frames.as_slice()).unwrap_or(&[])
    }

    /// Port of `PetDefinition.validate()`: returns non-fatal warnings, `Err` for problems
    /// that make the pet unusable. Every rule and message shape from the Swift version:
    /// id charset, non-empty name, single-char palette keys, reserved transparency chars,
    /// #RRGGBB/#RRGGBBAA colors, frames.idle required, uniform grid (min 4, max 64),
    /// every used char in palette, unknown state keys warned, unused palette keys warned,
    /// missing states warned with their fallback, duplicate problems deduped in order.
    pub fn validate(&self) -> Result<Vec<String>, ValidationError> {
        let mut problems: Vec<String> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();

        if self.id.trim().is_empty() {
            problems.push("`id` must not be empty".to_string());
        }
        if self
            .id
            .chars()
            .any(|c| !(c.is_alphabetic() || c.is_numeric() || c == '-' || c == '_'))
        {
            problems.push(format!(
                "`id` may only contain letters, digits, '-' and '_' (got \"{}\")",
                self.id
            ));
        }
        if self.name.trim().is_empty() {
            problems.push("`name` must not be empty".to_string());
        }

        // Palette characters in (deterministic) palette key order, plus a set for lookups.
        let mut palette_chars: Vec<char> = Vec::new();
        for (key, hex) in &self.palette {
            let mut chars = key.chars();
            let character = match (chars.next(), chars.next()) {
                (Some(character), None) => character,
                _ => {
                    problems.push(format!("palette key \"{key}\" must be exactly one character"));
                    continue;
                }
            };
            if TRANSPARENT_CHARS.contains(&character) {
                problems.push(format!("palette key \"{key}\" is reserved for transparency"));
            }
            if PixelColor::parse(hex).is_none() {
                problems.push(format!(
                    "palette[\"{key}\"] = \"{hex}\" is not a #RRGGBB / #RRGGBBAA color"
                ));
            }
            palette_chars.push(character);
        }
        let palette_set: HashSet<char> = palette_chars.iter().copied().collect();

        let idle_frames = match self.frames.get("idle") {
            Some(frames) if !frames.is_empty() => frames,
            _ => {
                problems.push("`frames.idle` is required and must contain at least one frame".to_string());
                return Err(ValidationError { problems });
            }
        };
        let expected_width = match idle_frames[0].first() {
            Some(row) => row.chars().count(),
            None => {
                problems.push("`frames.idle[0]` has no rows".to_string());
                return Err(ValidationError { problems });
            }
        };
        let expected_height = idle_frames[0].len();
        if expected_width < MIN_GRID_SIDE || expected_height < MIN_GRID_SIDE {
            problems.push(format!(
                "grid must be at least {MIN_GRID_SIDE}x{MIN_GRID_SIDE} (got {expected_width}x{expected_height})"
            ));
        }
        if expected_width > MAX_GRID_SIDE || expected_height > MAX_GRID_SIDE {
            problems.push(format!(
                "grid must be at most {MAX_GRID_SIDE}x{MAX_GRID_SIDE} (got {expected_width}x{expected_height})"
            ));
        }

        let known_states = PetState::ALL
            .iter()
            .map(|state| state.raw())
            .collect::<Vec<_>>()
            .join(", ");
        let mut used_characters: HashSet<char> = HashSet::new();
        for (state_key, state_frames) in &self.frames {
            if !PetState::ALL.iter().any(|state| state.raw() == state_key) {
                warnings.push(format!(
                    "frames.{state_key}: unknown state, ignored (known: {known_states})"
                ));
                continue;
            }
            if state_frames.is_empty() {
                warnings.push(format!("frames.{state_key}: empty, will fall back"));
                continue;
            }
            for (frame_index, frame) in state_frames.iter().enumerate() {
                if frame.len() != expected_height {
                    problems.push(format!(
                        "frames.{state_key}[{frame_index}] has {} rows, expected {expected_height}",
                        frame.len()
                    ));
                }
                for (row_index, row) in frame.iter().enumerate() {
                    let columns = row.chars().count();
                    if columns != expected_width {
                        problems.push(format!(
                            "frames.{state_key}[{frame_index}] row {row_index} has {columns} columns, expected {expected_width}"
                        ));
                    }
                    for character in row.chars() {
                        if TRANSPARENT_CHARS.contains(&character) {
                            continue;
                        }
                        used_characters.insert(character);
                        if !palette_set.contains(&character) {
                            problems.push(format!(
                                "frames.{state_key}[{frame_index}] row {row_index} uses \"{character}\" which is not in the palette"
                            ));
                        }
                    }
                }
            }
        }

        for character in &palette_chars {
            if !used_characters.contains(character) {
                warnings.push(format!("palette key \"{character}\" is never used"));
            }
        }
        for state in PetState::ALL {
            let missing = match self.frames.get(state.raw()) {
                None => true,
                Some(frames) => frames.is_empty(),
            };
            if missing && state != PetState::Idle {
                let fallback = state
                    .fallback_states()
                    .first()
                    .map(|fallback| fallback.raw())
                    .unwrap_or("idle");
                warnings.push(format!("no frames for {}; falling back to {fallback}", state.raw()));
            }
        }

        // Duplicate problems are noisy when a whole frame is the wrong width; keep unique, ordered.
        let mut seen: HashSet<String> = HashSet::new();
        let unique_problems: Vec<String> = problems
            .into_iter()
            .filter(|problem| seen.insert(problem.clone()))
            .collect();
        if !unique_problems.is_empty() {
            return Err(ValidationError { problems: unique_problems });
        }
        Ok(warnings)
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
        let mut text = hex.trim();
        if let Some(rest) = text.strip_prefix('#') {
            text = rest;
        }
        let digits = text.chars().count();
        if digits != 6 && digits != 8 {
            return None;
        }
        // u64::from_str_radix tolerates a leading sign; require pure ASCII hex digits.
        if !text.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let value = u64::from_str_radix(text, 16).ok()?;
        Some(if digits == 6 {
            PixelColor {
                r: ((value >> 16) & 0xFF) as u8,
                g: ((value >> 8) & 0xFF) as u8,
                b: (value & 0xFF) as u8,
                a: 255,
            }
        } else {
            PixelColor {
                r: ((value >> 24) & 0xFF) as u8,
                g: ((value >> 16) & 0xFF) as u8,
                b: ((value >> 8) & 0xFF) as u8,
                a: (value & 0xFF) as u8,
            }
        })
    }
}

/// Palette resolved to characters, ready for rendering (invalid entries skipped).
pub struct ResolvedPalette {
    pub colors: std::collections::HashMap<char, PixelColor>,
}

impl ResolvedPalette {
    pub fn new(definition: &PetDefinition) -> ResolvedPalette {
        let mut colors: HashMap<char, PixelColor> = HashMap::new();
        for (key, hex) in &definition.palette {
            let mut chars = key.chars();
            if let (Some(character), None) = (chars.next(), chars.next()) {
                if let Some(color) = PixelColor::parse(hex) {
                    colors.insert(character, color);
                }
            }
        }
        ResolvedPalette { colors }
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
        let mut library = PetLibrary { pets: Vec::new(), load_problems: Vec::new() };
        let mut pets_by_id: HashMap<String, LoadedPet> = HashMap::new();
        let mut order: Vec<String> = Vec::new();

        for (file_name, source) in BUILT_IN_PET_SOURCES {
            let definition = match PetDefinition::decode(source.as_bytes()) {
                Ok(definition) => definition,
                Err(error) => {
                    crate::logging::eprint_line(&format!(
                        "claude-airou: built-in pet {file_name} is invalid: {error}"
                    ));
                    continue;
                }
            };
            if let Err(error) = definition.validate() {
                crate::logging::eprint_line(&format!(
                    "claude-airou: built-in pet {file_name} is invalid: {error}"
                ));
                continue;
            }
            order.push(definition.id.clone());
            pets_by_id.insert(definition.id.clone(), LoadedPet { definition, source_path: None });
        }

        if let Ok(entries) = std::fs::read_dir(user_pets_dir) {
            let mut files: Vec<(String, PathBuf)> = entries
                .flatten()
                .filter_map(|entry| {
                    let path = entry.path();
                    let name = path.file_name()?.to_string_lossy().to_string();
                    // Swift lists with .skipsHiddenFiles.
                    if name.starts_with('.') {
                        return None;
                    }
                    Some((name, path))
                })
                .collect();
            files.sort_by(|a, b| a.0.cmp(&b.0));
            for (file_name, path) in files {
                let is_json = path
                    .extension()
                    .map(|extension| extension.to_string_lossy().to_lowercase() == "json")
                    .unwrap_or(false);
                if !is_json {
                    continue;
                }
                let loaded = PetDefinition::load(&path).and_then(|definition| {
                    definition.validate().map_err(|error| error.to_string())?;
                    Ok(definition)
                });
                match loaded {
                    Ok(definition) => {
                        if !pets_by_id.contains_key(&definition.id) {
                            order.push(definition.id.clone());
                        }
                        pets_by_id.insert(
                            definition.id.clone(),
                            LoadedPet { definition, source_path: Some(path) },
                        );
                    }
                    Err(why) => library.load_problems.push(format!("{file_name}: {why}")),
                }
            }
        }

        library.pets = order.iter().filter_map(|id| pets_by_id.remove(id)).collect();
        library
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(rows: &[&str]) -> Vec<String> {
        rows.iter().map(|row| row.to_string()).collect()
    }

    fn minimal_pet() -> PetDefinition {
        let mut palette = BTreeMap::new();
        palette.insert("k".to_string(), "#112233".to_string());
        let mut frames = BTreeMap::new();
        frames.insert("idle".to_string(), vec![rows(&["kkkk", "k..k", "k..k", "kkkk"])]);
        PetDefinition {
            id: "test-pet".to_string(),
            name: "Test".to_string(),
            species: "test".to_string(),
            description: None,
            author: None,
            fps: None,
            palette,
            phrases: None,
            frames,
        }
    }

    #[test]
    fn all_built_ins_decode_and_validate() {
        for (file_name, source) in BUILT_IN_PET_SOURCES {
            let definition = PetDefinition::decode(source.as_bytes())
                .unwrap_or_else(|error| panic!("{file_name} does not decode: {error}"));
            definition
                .validate()
                .unwrap_or_else(|error| panic!("{file_name} does not validate: {error}"));
        }
    }

    #[test]
    fn built_ins_keep_menu_order_in_library() {
        let library = PetLibrary::load_from(std::path::Path::new("/nonexistent/pets/dir"));
        let ids: Vec<&str> = library.pets.iter().map(|pet| pet.definition.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "airou-felyne",
                "mochi-cat",
                "quackers-duck",
                "boo-ghost",
                "jelly-slime",
                "bolt-robot",
                "inky-octopus",
                "clawd-claude",
            ]
        );
        assert!(library.pets.iter().all(|pet| pet.is_built_in()));
        assert!(library.load_problems.is_empty());
    }

    #[test]
    fn validate_reports_exact_problem_strings() {
        let mut pet = minimal_pet();
        pet.id = "bad id!".to_string();
        pet.name = "   ".to_string();
        pet.palette.insert("kk".to_string(), "#000000".to_string());
        pet.palette.insert(".".to_string(), "#000000".to_string());
        pet.palette.insert("a".to_string(), "green".to_string());
        pet.frames
            .insert("idle".to_string(), vec![rows(&["aaaz", "aaa", "aaaa", "aaaa"])]);

        let error = pet.validate().unwrap_err();
        let expected = vec![
            "`id` may only contain letters, digits, '-' and '_' (got \"bad id!\")".to_string(),
            "`name` must not be empty".to_string(),
            "palette key \".\" is reserved for transparency".to_string(),
            "palette[\"a\"] = \"green\" is not a #RRGGBB / #RRGGBBAA color".to_string(),
            "palette key \"kk\" must be exactly one character".to_string(),
            "frames.idle[0] row 0 uses \"z\" which is not in the palette".to_string(),
            "frames.idle[0] row 1 has 3 columns, expected 4".to_string(),
        ];
        assert_eq!(error.problems, expected);
    }

    #[test]
    fn validate_requires_idle_frames() {
        let mut pet = minimal_pet();
        pet.frames.clear();
        pet.frames
            .insert("working".to_string(), vec![rows(&["kkkk", "kkkk", "kkkk", "kkkk"])]);
        let error = pet.validate().unwrap_err();
        assert_eq!(
            error.problems.last().unwrap(),
            "`frames.idle` is required and must contain at least one frame"
        );

        let mut pet = minimal_pet();
        pet.frames.insert("idle".to_string(), vec![vec![]]);
        let error = pet.validate().unwrap_err();
        assert_eq!(error.problems.last().unwrap(), "`frames.idle[0]` has no rows");
    }

    #[test]
    fn validate_grid_bounds() {
        let mut pet = minimal_pet();
        pet.frames.insert("idle".to_string(), vec![rows(&["kk", "kk"])]);
        let error = pet.validate().unwrap_err();
        assert!(error
            .problems
            .contains(&"grid must be at least 4x4 (got 2x2)".to_string()));

        let mut pet = minimal_pet();
        let wide = "k".repeat(65);
        pet.frames
            .insert("idle".to_string(), vec![vec![wide.clone(), wide.clone(), wide.clone(), wide]]);
        let error = pet.validate().unwrap_err();
        assert!(error
            .problems
            .contains(&"grid must be at most 64x64 (got 65x4)".to_string()));
    }

    #[test]
    fn validate_row_count_mismatch_string() {
        let mut pet = minimal_pet();
        pet.frames
            .insert("thinking".to_string(), vec![rows(&["kkkk", "kkkk"])]);
        let error = pet.validate().unwrap_err();
        assert_eq!(
            error.problems,
            vec!["frames.thinking[0] has 2 rows, expected 4".to_string()]
        );
    }

    #[test]
    fn validate_warnings_match_swift() {
        let mut pet = minimal_pet();
        pet.palette.insert("u".to_string(), "#FFFFFF".to_string());
        pet.frames.insert("dance".to_string(), vec![rows(&["kkkk"])]);
        pet.frames.insert("done".to_string(), vec![]);
        let warnings = pet.validate().expect("warnings only");
        assert!(warnings.contains(&"frames.dance: unknown state, ignored (known: hello, idle, thinking, working, waiting_approval, needs_input, done, error)".to_string()));
        assert!(warnings.contains(&"frames.done: empty, will fall back".to_string()));
        assert!(warnings.contains(&"palette key \"u\" is never used".to_string()));
        assert!(warnings.contains(&"no frames for hello; falling back to done".to_string()));
        assert!(warnings.contains(&"no frames for working; falling back to thinking".to_string()));
        assert!(warnings.contains(&"no frames for waiting_approval; falling back to needs_input".to_string()));
        assert!(warnings.contains(&"no frames for needs_input; falling back to waiting_approval".to_string()));
        assert!(warnings.contains(&"no frames for done; falling back to idle".to_string()));
        assert!(warnings.contains(&"no frames for error; falling back to idle".to_string()));
        // Idle exists, so no warning about it.
        assert!(!warnings.iter().any(|warning| warning.starts_with("no frames for idle")));
    }

    #[test]
    fn validate_dedupes_problems_in_order() {
        let mut pet = minimal_pet();
        // Two frames with the same unknown character in the same position produce the
        // identical problem string twice per row; only unique strings survive, in order.
        pet.frames.insert(
            "idle".to_string(),
            vec![rows(&["zzzz", "kkkk", "kkkk", "kkkk"]), rows(&["kkkk", "kkkk", "kkkk", "zzzz"])],
        );
        let error = pet.validate().unwrap_err();
        assert_eq!(
            error.problems,
            vec![
                "frames.idle[0] row 0 uses \"z\" which is not in the palette".to_string(),
                "frames.idle[1] row 3 uses \"z\" which is not in the palette".to_string(),
            ]
        );
    }

    #[test]
    fn frames_for_follows_fallback_chain() {
        let mut pet = minimal_pet();
        let idle = rows(&["kkkk", "k..k", "k..k", "kkkk"]);
        let thinking = rows(&["....", "kkkk", "kkkk", "...."]);
        let done = rows(&["kk..", "kk..", "kk..", "kk.."]);
        pet.frames.insert("thinking".to_string(), vec![thinking.clone()]);
        pet.frames.insert("done".to_string(), vec![done.clone()]);
        pet.frames.insert("error".to_string(), vec![]);

        assert_eq!(pet.frames_for(PetState::Idle), &[idle.clone()]);
        // working -> thinking (first fallback present)
        assert_eq!(pet.frames_for(PetState::Working), &[thinking.clone()]);
        // hello -> done (first fallback present)
        assert_eq!(pet.frames_for(PetState::Hello), &[done.clone()]);
        // waiting_approval -> needs_input missing -> idle
        assert_eq!(pet.frames_for(PetState::WaitingApproval), &[idle.clone()]);
        // error defined but empty -> idle
        assert_eq!(pet.frames_for(PetState::Error), &[idle.clone()]);
    }

    #[test]
    fn frames_for_empty_when_even_idle_missing() {
        let mut pet = minimal_pet();
        pet.frames.clear();
        assert!(pet.frames_for(PetState::Idle).is_empty());
        assert!(pet.frames_for(PetState::Working).is_empty());
        assert_eq!(pet.grid_size(), (0, 0));
    }

    #[test]
    fn frames_per_second_clamps_like_swift() {
        let mut pet = minimal_pet();
        assert_eq!(pet.frames_per_second(), 3.0);
        pet.fps = Some(0.0);
        assert_eq!(pet.frames_per_second(), 3.0);
        pet.fps = Some(-2.0);
        assert_eq!(pet.frames_per_second(), 3.0);
        pet.fps = Some(0.1);
        assert_eq!(pet.frames_per_second(), 0.5);
        pet.fps = Some(30.0);
        assert_eq!(pet.frames_per_second(), 12.0);
        pet.fps = Some(4.0);
        assert_eq!(pet.frames_per_second(), 4.0);
        pet.fps = Some(f64::NAN);
        assert_eq!(pet.frames_per_second(), 3.0);
    }

    #[test]
    fn pet_phrases_falls_back_to_heart() {
        let mut pet = minimal_pet();
        assert_eq!(pet.pet_phrases(), vec!["♥".to_string()]);
        let mut phrases = BTreeMap::new();
        phrases.insert("pet".to_string(), Vec::<String>::new());
        pet.phrases = Some(phrases.clone());
        assert_eq!(pet.pet_phrases(), vec!["♥".to_string()]);
        phrases.insert("pet".to_string(), vec!["Purr…".to_string()]);
        pet.phrases = Some(phrases);
        assert_eq!(pet.pet_phrases(), vec!["Purr…".to_string()]);
    }

    #[test]
    fn grid_size_from_idle() {
        let pet = minimal_pet();
        assert_eq!(pet.grid_size(), (4, 4));
    }

    #[test]
    fn pixel_color_parse_edge_cases() {
        // Signs are rejected: only ASCII hex digits allowed.
        assert_eq!(PixelColor::parse("+ABCDE1"), None);
        assert_eq!(PixelColor::parse("+ABCDE"), None);
        assert_eq!(PixelColor::parse("-ABCDE"), None);
        assert_eq!(PixelColor::parse("#+ABCDE"), None);
        // Wrong lengths.
        assert_eq!(PixelColor::parse(""), None);
        assert_eq!(PixelColor::parse("#12345"), None);
        assert_eq!(PixelColor::parse("#1234567"), None);
        assert_eq!(PixelColor::parse("#123456789"), None);
        // Non-hex.
        assert_eq!(PixelColor::parse("GGGGGG"), None);
        // 6 digits, leading # optional, whitespace trimmed.
        assert_eq!(
            PixelColor::parse("#3a3f4b"),
            Some(PixelColor { r: 0x3a, g: 0x3f, b: 0x4b, a: 0xff })
        );
        assert_eq!(
            PixelColor::parse("  112233  "),
            Some(PixelColor { r: 0x11, g: 0x22, b: 0x33, a: 0xff })
        );
        // 8 digits carry alpha.
        assert_eq!(
            PixelColor::parse("#11223344"),
            Some(PixelColor { r: 0x11, g: 0x22, b: 0x33, a: 0x44 })
        );
        assert_eq!(
            PixelColor::parse("FFFFFFFF"),
            Some(PixelColor { r: 0xff, g: 0xff, b: 0xff, a: 0xff })
        );
        // A second '#' is not stripped.
        assert_eq!(PixelColor::parse("##AABBCC"), None);
    }

    #[test]
    fn resolved_palette_skips_invalid_entries() {
        let mut pet = minimal_pet();
        pet.palette.insert("x".to_string(), "nope".to_string());
        pet.palette.insert("yy".to_string(), "#FFFFFF".to_string());
        pet.palette.insert("z".to_string(), "#00FF0080".to_string());
        let palette = ResolvedPalette::new(&pet);
        assert_eq!(palette.colors.len(), 2);
        assert_eq!(
            palette.colors.get(&'k'),
            Some(&PixelColor { r: 0x11, g: 0x22, b: 0x33, a: 0xff })
        );
        assert_eq!(
            palette.colors.get(&'z'),
            Some(&PixelColor { r: 0x00, g: 0xff, b: 0x00, a: 0x80 })
        );
        assert!(palette.colors.get(&'x').is_none());
    }

    #[test]
    fn library_loads_user_pets_sorted_with_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();

        let mut custom = minimal_pet();
        custom.id = "zeta-pet".to_string();
        custom.name = "Zeta".to_string();
        std::fs::write(path.join("b-custom.json"), serde_json::to_vec(&custom).unwrap()).unwrap();

        // Overrides the built-in mochi-cat but keeps its menu position.
        let mut override_pet = minimal_pet();
        override_pet.id = "mochi-cat".to_string();
        override_pet.name = "Custom Mochi".to_string();
        std::fs::write(path.join("a-override.json"), serde_json::to_vec(&override_pet).unwrap()).unwrap();

        std::fs::write(path.join("broken.json"), b"{ not json").unwrap();

        let mut invalid = minimal_pet();
        invalid.id = "invalid-pet".to_string();
        invalid.frames.clear();
        std::fs::write(path.join("invalid-pet.json"), serde_json::to_vec(&invalid).unwrap()).unwrap();

        // Hidden and non-json files are skipped entirely.
        let mut hidden = minimal_pet();
        hidden.id = "hidden-pet".to_string();
        std::fs::write(path.join(".hidden.json"), serde_json::to_vec(&hidden).unwrap()).unwrap();
        std::fs::write(path.join("notes.txt"), b"not a pet").unwrap();

        let library = PetLibrary::load_from(path);

        let ids: Vec<&str> = library.pets.iter().map(|pet| pet.definition.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "airou-felyne",
                "mochi-cat",
                "quackers-duck",
                "boo-ghost",
                "jelly-slime",
                "bolt-robot",
                "inky-octopus",
                "clawd-claude",
                "zeta-pet",
            ]
        );

        let mochi = library.pet_with_id("mochi-cat").unwrap();
        assert_eq!(mochi.definition.name, "Custom Mochi");
        assert!(!mochi.is_built_in());
        assert_eq!(mochi.source_path.as_deref(), Some(path.join("a-override.json").as_path()));

        assert!(library.pet_with_id("hidden-pet").is_none());

        assert_eq!(library.load_problems.len(), 2);
        assert!(library.load_problems[0].starts_with("broken.json: "));
        assert!(library.load_problems[1].starts_with("invalid-pet.json: "));
        assert!(library.load_problems[1]
            .contains("`frames.idle` is required and must contain at least one frame"));
    }

    #[test]
    fn resolve_selected_prefers_configured_then_first() {
        let library = PetLibrary::load_from(std::path::Path::new("/nonexistent/pets/dir"));
        assert_eq!(
            library.resolve_selected(Some("boo-ghost")).unwrap().definition.id,
            "boo-ghost"
        );
        assert_eq!(
            library.resolve_selected(Some("missing")).unwrap().definition.id,
            "airou-felyne"
        );
        assert_eq!(library.resolve_selected(None).unwrap().definition.id, "airou-felyne");
    }

    #[test]
    fn load_reports_readable_errors() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.json");
        let error = PetDefinition::load(&missing).unwrap_err();
        assert!(error.starts_with("could not read file: "));

        let bad = dir.path().join("bad.json");
        std::fs::write(&bad, b"[1, 2").unwrap();
        let error = PetDefinition::load(&bad).unwrap_err();
        assert!(error.starts_with("invalid pet JSON: "));
    }
}
