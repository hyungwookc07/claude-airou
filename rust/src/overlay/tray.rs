//! Menu-bar (tray) icon and menu for the overlay, via `tray-icon`/`muda`.
//! Menu structure per the v0.1 spec (a functional subset of Swift's
//! `AppDelegate.populateMenu`): header + usage line, Pet submenu (built-ins + user
//! pets, checked selection, "Reload pets"), Size (Small 3 / Medium 5 / Large 7),
//! Gauge submenu, the three toggles, Reset position, Quit. Session pinning and the
//! fan-out toggle are deferred.

use crate::model::GaugeMetric;
use crate::pets::PetLibrary;
use tray_icon::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};

/// Everything the tray menu needs to render; also acts as the rebuild signature.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuModel {
    pub header: String,
    pub usage_line: Option<String>,
    /// (id, title, is_selected) per pet, in library order; title carries "  (custom)".
    pub pets: Vec<(String, String, bool)>,
    pub pixel_scale: f64,
    pub gauge_metric: GaugeMetric,
    pub bubbles_hidden: bool,
    pub click_through: bool,
    pub pet_hidden: bool,
}

impl MenuModel {
    pub fn pet_rows(library: &PetLibrary, selected_pet_id: &str) -> Vec<(String, String, bool)> {
        library
            .pets
            .iter()
            .map(|pet| {
                let suffix = if pet.is_built_in() { "" } else { "  (custom)" };
                (
                    pet.definition.id.clone(),
                    format!("{}{suffix}", pet.definition.name),
                    pet.definition.id == selected_pet_id,
                )
            })
            .collect()
    }
}

/// What a clicked menu item means. Menu item ids round-trip through these.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuAction {
    SelectPet(String),
    SelectSize(f64),
    SelectGauge(GaugeMetric),
    ToggleBubbles,
    ToggleClickThrough,
    TogglePetHidden,
    ReloadPets,
    ResetPosition,
    Quit,
}

pub const SIZE_OPTIONS: [(&str, f64); 3] = [("Small", 3.0), ("Medium", 5.0), ("Large", 7.0)];

pub const GAUGE_OPTIONS: [GaugeMetric; 4] = [
    GaugeMetric::ContextRemaining,
    GaugeMetric::FiveHourRemaining,
    GaugeMetric::SevenDayRemaining,
    GaugeMetric::Off,
];

/// Raw value shared with the config file / Swift (`GaugeMetric` raw values).
pub fn gauge_raw(metric: GaugeMetric) -> &'static str {
    match metric {
        GaugeMetric::ContextRemaining => "context_remaining",
        GaugeMetric::FiveHourRemaining => "five_hour_remaining",
        GaugeMetric::SevenDayRemaining => "seven_day_remaining",
        GaugeMetric::Off => "off",
    }
}

pub fn gauge_from_raw(raw: &str) -> Option<GaugeMetric> {
    GAUGE_OPTIONS.into_iter().find(|metric| gauge_raw(*metric) == raw)
}

/// Menu titles, identical to Swift's `GaugeMetric.menuTitle`.
pub fn gauge_menu_title(metric: GaugeMetric) -> &'static str {
    match metric {
        GaugeMetric::ContextRemaining => "Context window remaining",
        GaugeMetric::FiveHourRemaining => "5-hour limit remaining",
        GaugeMetric::SevenDayRemaining => "7-day limit remaining",
        GaugeMetric::Off => "Off",
    }
}

pub fn menu_id_for(action: &MenuAction) -> String {
    match action {
        MenuAction::SelectPet(id) => format!("pet:{id}"),
        MenuAction::SelectSize(scale) => format!("size:{scale}"),
        MenuAction::SelectGauge(metric) => format!("gauge:{}", gauge_raw(*metric)),
        MenuAction::ToggleBubbles => "toggle:bubbles".to_string(),
        MenuAction::ToggleClickThrough => "toggle:click-through".to_string(),
        MenuAction::TogglePetHidden => "toggle:pet-hidden".to_string(),
        MenuAction::ReloadPets => "reload-pets".to_string(),
        MenuAction::ResetPosition => "reset-position".to_string(),
        MenuAction::Quit => "quit".to_string(),
    }
}

pub fn action_for_menu_id(id: &str) -> Option<MenuAction> {
    if let Some(pet_id) = id.strip_prefix("pet:") {
        return Some(MenuAction::SelectPet(pet_id.to_string()));
    }
    if let Some(scale) = id.strip_prefix("size:") {
        return scale.parse::<f64>().ok().map(MenuAction::SelectSize);
    }
    if let Some(raw) = id.strip_prefix("gauge:") {
        return gauge_from_raw(raw).map(MenuAction::SelectGauge);
    }
    match id {
        "toggle:bubbles" => Some(MenuAction::ToggleBubbles),
        "toggle:click-through" => Some(MenuAction::ToggleClickThrough),
        "toggle:pet-hidden" => Some(MenuAction::TogglePetHidden),
        "reload-pets" => Some(MenuAction::ReloadPets),
        "reset-position" => Some(MenuAction::ResetPosition),
        "quit" => Some(MenuAction::Quit),
        _ => None,
    }
}

/// The size option nearest to `pixel_scale`, so hand-edited configs still show a check
/// (mirrors Swift's nearest-scale logic).
pub fn nearest_size_option(pixel_scale: f64) -> f64 {
    SIZE_OPTIONS
        .iter()
        .min_by(|a, b| {
            (a.1 - pixel_scale)
                .abs()
                .partial_cmp(&(b.1 - pixel_scale).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|option| option.1)
        .unwrap_or(5.0)
}

/// Builds the whole tray menu from a `MenuModel`. Any muda error degrades to a menu
/// with whatever items made it in (never panics on the tick path).
pub fn build_menu(model: &MenuModel) -> Menu {
    let menu = Menu::new();

    let header = MenuItem::with_id("header", &model.header, false, None);
    let _ = menu.append(&header);
    if let Some(usage_line) = &model.usage_line {
        let usage = MenuItem::with_id("usage", usage_line, false, None);
        let _ = menu.append(&usage);
    }
    let _ = menu.append(&PredefinedMenuItem::separator());

    // Pet submenu.
    let pet_menu = Submenu::new("Pet", true);
    for (pet_id, title, selected) in &model.pets {
        let item = CheckMenuItem::with_id(
            menu_id_for(&MenuAction::SelectPet(pet_id.clone())),
            title,
            true,
            *selected,
            None,
        );
        let _ = pet_menu.append(&item);
    }
    let _ = pet_menu.append(&PredefinedMenuItem::separator());
    let _ = pet_menu.append(&MenuItem::with_id(
        menu_id_for(&MenuAction::ReloadPets),
        "Reload pets",
        true,
        None,
    ));
    let _ = menu.append(&pet_menu);

    // Size submenu.
    let size_menu = Submenu::new("Size", true);
    let nearest = nearest_size_option(model.pixel_scale);
    for (label, scale) in SIZE_OPTIONS {
        let item = CheckMenuItem::with_id(
            menu_id_for(&MenuAction::SelectSize(scale)),
            label,
            true,
            scale == nearest,
            None,
        );
        let _ = size_menu.append(&item);
    }
    let _ = menu.append(&size_menu);

    // Gauge submenu.
    let gauge_menu = Submenu::new("Gauge", true);
    for metric in GAUGE_OPTIONS {
        let item = CheckMenuItem::with_id(
            menu_id_for(&MenuAction::SelectGauge(metric)),
            gauge_menu_title(metric),
            true,
            metric == model.gauge_metric,
            None,
        );
        let _ = gauge_menu.append(&item);
    }
    let _ = menu.append(&gauge_menu);

    let _ = menu.append(&PredefinedMenuItem::separator());

    let bubbles = CheckMenuItem::with_id(
        menu_id_for(&MenuAction::ToggleBubbles),
        "Hide speech bubbles",
        true,
        model.bubbles_hidden,
        None,
    );
    let _ = menu.append(&bubbles);
    let click_through = CheckMenuItem::with_id(
        menu_id_for(&MenuAction::ToggleClickThrough),
        "Click-through (ignore mouse)",
        true,
        model.click_through,
        None,
    );
    let _ = menu.append(&click_through);
    let hide = MenuItem::with_id(
        menu_id_for(&MenuAction::TogglePetHidden),
        if model.pet_hidden { "Show pet" } else { "Hide pet" },
        true,
        None,
    );
    let _ = menu.append(&hide);
    let _ = menu.append(&MenuItem::with_id(
        menu_id_for(&MenuAction::ResetPosition),
        "Reset position",
        true,
        None,
    ));

    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&MenuItem::with_id(
        menu_id_for(&MenuAction::Quit),
        "Quit Claude Airou",
        true,
        None,
    ));

    menu
}

/// A small paw-print bitmap, drawn programmatically (no assets): one big pad + three
/// toes, opaque black on transparent — used as a macOS template image so the menu bar
/// tints it correctly in light/dark mode.
pub fn paw_icon_rgba(size: u32) -> Vec<u8> {
    let mut rgba = vec![0u8; size as usize * size as usize * 4];
    let s = size as f64;
    // (cx, cy, rx, ry) as fractions of the icon size.
    let shapes: [(f64, f64, f64, f64); 4] = [
        (0.50, 0.66, 0.30, 0.24), // main pad
        (0.22, 0.32, 0.12, 0.14), // left toe
        (0.50, 0.24, 0.12, 0.14), // middle toe
        (0.78, 0.32, 0.12, 0.14), // right toe
    ];
    for y in 0..size {
        for x in 0..size {
            let fx = (x as f64 + 0.5) / s;
            let fy = (y as f64 + 0.5) / s;
            let inside = shapes.iter().any(|(cx, cy, rx, ry)| {
                let dx = (fx - cx) / rx;
                let dy = (fy - cy) / ry;
                dx * dx + dy * dy <= 1.0
            });
            if inside {
                let offset = (y as usize * size as usize + x as usize) * 4;
                rgba[offset] = 0;
                rgba[offset + 1] = 0;
                rgba[offset + 2] = 0;
                rgba[offset + 3] = 255;
            }
        }
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_ids_round_trip() {
        let actions = [
            MenuAction::SelectPet("airou-felyne".to_string()),
            MenuAction::SelectSize(3.0),
            MenuAction::SelectSize(7.0),
            MenuAction::SelectGauge(GaugeMetric::FiveHourRemaining),
            MenuAction::SelectGauge(GaugeMetric::Off),
            MenuAction::ToggleBubbles,
            MenuAction::ToggleClickThrough,
            MenuAction::TogglePetHidden,
            MenuAction::ReloadPets,
            MenuAction::ResetPosition,
            MenuAction::Quit,
        ];
        for action in actions {
            let id = menu_id_for(&action);
            assert_eq!(action_for_menu_id(&id), Some(action.clone()), "id {id}");
        }
        assert_eq!(action_for_menu_id("header"), None);
        assert_eq!(action_for_menu_id("usage"), None);
        assert_eq!(action_for_menu_id("gauge:bogus"), None);
        assert_eq!(action_for_menu_id("size:zzz"), None);
        assert_eq!(action_for_menu_id(""), None);
    }

    #[test]
    fn gauge_raw_values_match_config_serialization() {
        for metric in GAUGE_OPTIONS {
            let json = serde_json::to_string(&metric).unwrap();
            assert_eq!(json, format!("\"{}\"", gauge_raw(metric)));
            assert_eq!(gauge_from_raw(gauge_raw(metric)), Some(metric));
        }
    }

    #[test]
    fn gauge_titles_match_swift() {
        assert_eq!(gauge_menu_title(GaugeMetric::ContextRemaining), "Context window remaining");
        assert_eq!(gauge_menu_title(GaugeMetric::FiveHourRemaining), "5-hour limit remaining");
        assert_eq!(gauge_menu_title(GaugeMetric::SevenDayRemaining), "7-day limit remaining");
        assert_eq!(gauge_menu_title(GaugeMetric::Off), "Off");
    }

    #[test]
    fn nearest_size_marks_closest_option() {
        assert_eq!(nearest_size_option(3.0), 3.0);
        assert_eq!(nearest_size_option(5.0), 5.0);
        assert_eq!(nearest_size_option(7.0), 7.0);
        assert_eq!(nearest_size_option(1.0), 3.0);
        assert_eq!(nearest_size_option(6.4), 7.0);
        assert_eq!(nearest_size_option(12.0), 7.0);
    }

    #[test]
    fn pet_rows_mark_selection_and_custom_suffix() {
        let library = PetLibrary::load_from(std::path::Path::new("/nonexistent"));
        let rows = MenuModel::pet_rows(&library, "mochi-cat");
        assert_eq!(rows.len(), 8);
        assert!(rows.iter().all(|(_, title, _)| !title.contains("(custom)")));
        let selected: Vec<&str> = rows
            .iter()
            .filter(|(_, _, selected)| *selected)
            .map(|(id, _, _)| id.as_str())
            .collect();
        assert_eq!(selected, vec!["mochi-cat"]);
    }

    #[test]
    fn paw_icon_has_opaque_center_and_transparent_corners() {
        let size = 22u32;
        let rgba = paw_icon_rgba(size);
        assert_eq!(rgba.len(), (size * size * 4) as usize);
        let alpha_at = |x: u32, y: u32| rgba[((y * size + x) * 4 + 3) as usize];
        assert_eq!(alpha_at(0, 0), 0);
        assert_eq!(alpha_at(size - 1, 0), 0);
        assert_eq!(alpha_at(0, size - 1), 0);
        // Main pad centre is opaque.
        assert_eq!(alpha_at(size / 2, (size as f64 * 0.66) as u32), 255);
    }
}
