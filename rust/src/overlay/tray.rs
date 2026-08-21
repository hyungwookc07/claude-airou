//! Menu-bar (tray) icon and menu for the overlay, via `tray-icon`/`muda`. The same menu
//! doubles as the pet's right-click context menu. Structure and order follow Swift's
//! `AppDelegate.populateMenu` 1:1: header + usage line, Sessions (N) submenu (Automatic +
//! one pin entry per session), Gauge submenu (+ status line installer), the fan-out
//! toggle, Pet submenu (built-ins + user pets, Reload, Open folder), Size, the three
//! toggles, Reset position, hooks installer, hook log, Quit.

use crate::model::GaugeMetric;
use crate::pets::PetLibrary;
use tray_icon::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};

/// One row of the Sessions submenu.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionMenuRow {
    pub session_id: String,
    /// "<project> — <state label>"
    pub title: String,
    pub is_pinned: bool,
}

/// Everything the tray menu needs to render; also acts as the rebuild signature.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuModel {
    pub header: String,
    pub usage_line: Option<String>,
    /// Sessions in overlay order (newest first); empty hides the submenu.
    pub sessions: Vec<SessionMenuRow>,
    /// True when no session is pinned ("Automatic" is checked).
    pub is_following_automatically: bool,
    /// (id, title, is_selected) per pet, in library order; title carries "  (custom)".
    pub pets: Vec<(String, String, bool)>,
    pub pixel_scale: f64,
    pub gauge_metric: GaugeMetric,
    pub always_expanded: bool,
    pub bubbles_hidden: bool,
    pub click_through: bool,
    pub pet_hidden: bool,
    /// Whether the effort aura behind the pet is switched off.
    pub effort_aura_hidden: bool,
    /// Whether the shadow clone per working subagent is switched off.
    pub agent_shadows_hidden: bool,
    /// Whether the login item is registered (setup leaves it off by default).
    pub start_at_login: bool,
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
    PinSession(String),
    FollowSessionsAutomatically,
    SelectPet(String),
    SelectSize(f64),
    SelectGauge(GaugeMetric),
    InstallStatusLine,
    ToggleAlwaysExpanded,
    ToggleBubbles,
    ToggleEffortAura,
    ToggleAgentShadows,
    ToggleClickThrough,
    TogglePetHidden,
    ReloadPets,
    OpenPetsFolder,
    ResetPosition,
    InstallHooks,
    InstallMcp,
    ToggleStartAtLogin,
    OpenHookLog,
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
        MenuAction::PinSession(id) => format!("session:{id}"),
        MenuAction::FollowSessionsAutomatically => "sessions:automatic".to_string(),
        MenuAction::SelectPet(id) => format!("pet:{id}"),
        MenuAction::SelectSize(scale) => format!("size:{scale}"),
        MenuAction::SelectGauge(metric) => format!("gauge:{}", gauge_raw(*metric)),
        MenuAction::InstallStatusLine => "install-statusline".to_string(),
        MenuAction::ToggleEffortAura => "toggle-effort-aura".to_string(),
        MenuAction::ToggleAgentShadows => "toggle-agent-shadows".to_string(),
        MenuAction::ToggleAlwaysExpanded => "toggle:always-expanded".to_string(),
        MenuAction::ToggleBubbles => "toggle:bubbles".to_string(),
        MenuAction::ToggleClickThrough => "toggle:click-through".to_string(),
        MenuAction::TogglePetHidden => "toggle:pet-hidden".to_string(),
        MenuAction::ReloadPets => "reload-pets".to_string(),
        MenuAction::OpenPetsFolder => "open-pets-folder".to_string(),
        MenuAction::ResetPosition => "reset-position".to_string(),
        MenuAction::InstallHooks => "install-hooks".to_string(),
        MenuAction::InstallMcp => "install-mcp".to_string(),
        MenuAction::ToggleStartAtLogin => "toggle-start-at-login".to_string(),
        MenuAction::OpenHookLog => "open-hook-log".to_string(),
        MenuAction::Quit => "quit".to_string(),
    }
}

pub fn action_for_menu_id(id: &str) -> Option<MenuAction> {
    if let Some(session_id) = id.strip_prefix("session:") {
        return Some(MenuAction::PinSession(session_id.to_string()));
    }
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
        "sessions:automatic" => Some(MenuAction::FollowSessionsAutomatically),
        "install-statusline" => Some(MenuAction::InstallStatusLine),
        "toggle-effort-aura" => Some(MenuAction::ToggleEffortAura),
        "toggle-agent-shadows" => Some(MenuAction::ToggleAgentShadows),
        "toggle:always-expanded" => Some(MenuAction::ToggleAlwaysExpanded),
        "toggle:bubbles" => Some(MenuAction::ToggleBubbles),
        "toggle:click-through" => Some(MenuAction::ToggleClickThrough),
        "toggle:pet-hidden" => Some(MenuAction::TogglePetHidden),
        "reload-pets" => Some(MenuAction::ReloadPets),
        "open-pets-folder" => Some(MenuAction::OpenPetsFolder),
        "reset-position" => Some(MenuAction::ResetPosition),
        "install-hooks" => Some(MenuAction::InstallHooks),
        "install-mcp" => Some(MenuAction::InstallMcp),
        "toggle-start-at-login" => Some(MenuAction::ToggleStartAtLogin),
        "open-hook-log" => Some(MenuAction::OpenHookLog),
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

/// The live tray menu: the muda handle for every item whose text or check mark can
/// change, so ticks refresh them in place. Replacing the whole menu (`set_menu`) while
/// the user has it open detaches the open copy — the rows stay visible but every click
/// goes dead — and the header/usage text changes on almost every tick while a session is
/// busy, so "replace on any change" had the menu broken more often than not.
pub struct TrayMenu {
    pub menu: Menu,
    header: MenuItem,
    usage: Option<MenuItem>,
    automatic: Option<CheckMenuItem>,
    session_rows: Vec<CheckMenuItem>,
    gauge_items: Vec<(GaugeMetric, CheckMenuItem)>,
    expand: CheckMenuItem,
    pet_items: Vec<CheckMenuItem>,
    size_items: Vec<(f64, CheckMenuItem)>,
    bubbles: CheckMenuItem,
    aura: CheckMenuItem,
    shadows: CheckMenuItem,
    click_through: CheckMenuItem,
    hide_pet: MenuItem,
    start_at_login: CheckMenuItem,
}

/// True when `next` differs from `previous` only in texts and check marks — everything
/// `TrayMenu::apply` changes in place. Rows appearing or disappearing (sessions, pets,
/// the usage line) still need a rebuild.
pub fn same_menu_structure(previous: &MenuModel, next: &MenuModel) -> bool {
    previous.usage_line.is_some() == next.usage_line.is_some()
        && previous.sessions.len() == next.sessions.len()
        && previous
            .sessions
            .iter()
            .zip(&next.sessions)
            .all(|(a, b)| a.session_id == b.session_id)
        && previous.pets.len() == next.pets.len()
        && previous
            .pets
            .iter()
            .zip(&next.pets)
            .all(|(a, b)| a.0 == b.0 && a.1 == b.1)
}

impl TrayMenu {
    /// Builds the whole tray menu from a `MenuModel` (Swift `populateMenu` order),
    /// keeping the handles for `apply`. Any muda error degrades to a menu with whatever
    /// items made it in (never panics on the tick path).
    pub fn build(model: &MenuModel) -> TrayMenu {
        let menu = Menu::new();

        let header = MenuItem::with_id("header", &model.header, false, None);
        let _ = menu.append(&header);
        let usage = model.usage_line.as_ref().map(|usage_line| {
            let item = MenuItem::with_id("usage", usage_line, false, None);
            let _ = menu.append(&item);
            item
        });

        // Sessions submenu: Automatic + one pin entry per session.
        let mut automatic = None;
        let mut session_rows = Vec::with_capacity(model.sessions.len());
        if !model.sessions.is_empty() {
            let sessions_menu = Submenu::new(format!("Sessions ({})", model.sessions.len()), true);
            let automatic_item = CheckMenuItem::with_id(
                menu_id_for(&MenuAction::FollowSessionsAutomatically),
                "Automatic (approval > busy > recent)",
                true,
                model.is_following_automatically,
                None,
            );
            let _ = sessions_menu.append(&automatic_item);
            automatic = Some(automatic_item);
            let _ = sessions_menu.append(&PredefinedMenuItem::separator());
            for row in &model.sessions {
                let item = CheckMenuItem::with_id(
                    menu_id_for(&MenuAction::PinSession(row.session_id.clone())),
                    &row.title,
                    true,
                    row.is_pinned,
                    None,
                );
                let _ = sessions_menu.append(&item);
                session_rows.push(item);
            }
            let _ = menu.append(&sessions_menu);
        }

        // Gauge submenu (+ status line installer).
        let gauge_menu = Submenu::new("Gauge", true);
        let mut gauge_items = Vec::with_capacity(GAUGE_OPTIONS.len());
        for metric in GAUGE_OPTIONS {
            let item = CheckMenuItem::with_id(
                menu_id_for(&MenuAction::SelectGauge(metric)),
                gauge_menu_title(metric),
                true,
                metric == model.gauge_metric,
                None,
            );
            let _ = gauge_menu.append(&item);
            gauge_items.push((metric, item));
        }
        let _ = gauge_menu.append(&PredefinedMenuItem::separator());
        let _ = gauge_menu.append(&MenuItem::with_id(
            menu_id_for(&MenuAction::InstallStatusLine),
            "Feed from Claude Code status line…",
            true,
            None,
        ));
        let _ = menu.append(&gauge_menu);

        let expand = CheckMenuItem::with_id(
            menu_id_for(&MenuAction::ToggleAlwaysExpanded),
            "Show all sessions side by side",
            true,
            model.always_expanded,
            None,
        );
        let _ = menu.append(&expand);

        let _ = menu.append(&PredefinedMenuItem::separator());

        // Pet submenu.
        let pet_menu = Submenu::new("Pet", true);
        let mut pet_items = Vec::with_capacity(model.pets.len());
        for (pet_id, title, selected) in &model.pets {
            let item = CheckMenuItem::with_id(
                menu_id_for(&MenuAction::SelectPet(pet_id.clone())),
                title,
                true,
                *selected,
                None,
            );
            let _ = pet_menu.append(&item);
            pet_items.push(item);
        }
        let _ = pet_menu.append(&PredefinedMenuItem::separator());
        let _ = pet_menu.append(&MenuItem::with_id(
            menu_id_for(&MenuAction::ReloadPets),
            "Reload pets",
            true,
            None,
        ));
        let _ = pet_menu.append(&MenuItem::with_id(
            menu_id_for(&MenuAction::OpenPetsFolder),
            "Open pets folder…",
            true,
            None,
        ));
        let _ = menu.append(&pet_menu);

        // Size submenu.
        let size_menu = Submenu::new("Size", true);
        let nearest = nearest_size_option(model.pixel_scale);
        let mut size_items = Vec::with_capacity(SIZE_OPTIONS.len());
        for (label, scale) in SIZE_OPTIONS {
            let item = CheckMenuItem::with_id(
                menu_id_for(&MenuAction::SelectSize(scale)),
                label,
                true,
                scale == nearest,
                None,
            );
            let _ = size_menu.append(&item);
            size_items.push((scale, item));
        }
        let _ = menu.append(&size_menu);

        let _ = menu.append(&PredefinedMenuItem::separator());

        let bubbles = CheckMenuItem::with_id(
            menu_id_for(&MenuAction::ToggleBubbles),
            "Hide speech bubbles",
            true,
            model.bubbles_hidden,
            None,
        );
        let _ = menu.append(&bubbles);
        let aura = CheckMenuItem::with_id(
            menu_id_for(&MenuAction::ToggleEffortAura),
            "Hide effort aura",
            true,
            model.effort_aura_hidden,
            None,
        );
        let _ = menu.append(&aura);
        let shadows = CheckMenuItem::with_id(
            menu_id_for(&MenuAction::ToggleAgentShadows),
            "Hide agent shadows",
            true,
            model.agent_shadows_hidden,
            None,
        );
        let _ = menu.append(&shadows);
        let click_through = CheckMenuItem::with_id(
            menu_id_for(&MenuAction::ToggleClickThrough),
            "Click-through (ignore mouse)",
            true,
            model.click_through,
            None,
        );
        let _ = menu.append(&click_through);
        let hide_pet = MenuItem::with_id(
            menu_id_for(&MenuAction::TogglePetHidden),
            if model.pet_hidden { "Show pet" } else { "Hide pet" },
            true,
            None,
        );
        let _ = menu.append(&hide_pet);
        let _ = menu.append(&MenuItem::with_id(
            menu_id_for(&MenuAction::ResetPosition),
            "Reset position",
            true,
            None,
        ));

        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&MenuItem::with_id(
            menu_id_for(&MenuAction::InstallHooks),
            "Install Claude Code hooks…",
            true,
            None,
        ));
        let _ = menu.append(&MenuItem::with_id(
            menu_id_for(&MenuAction::InstallMcp),
            "Install MCP server for Claude chat…",
            true,
            None,
        ));
        let start_at_login = CheckMenuItem::with_id(
            menu_id_for(&MenuAction::ToggleStartAtLogin),
            "Start at login",
            true,
            model.start_at_login,
            None,
        );
        let _ = menu.append(&start_at_login);
        let _ = menu.append(&MenuItem::with_id(
            menu_id_for(&MenuAction::OpenHookLog),
            "Open hook log",
            true,
            None,
        ));

        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&MenuItem::with_id(
            menu_id_for(&MenuAction::Quit),
            "Quit Claude Airou",
            true,
            Some(tray_icon::menu::accelerator::Accelerator::new(
                Some(tray_icon::menu::accelerator::Modifiers::META),
                tray_icon::menu::accelerator::Code::KeyQ,
            )),
        ));

        TrayMenu {
            menu,
            header,
            usage,
            automatic,
            session_rows,
            gauge_items,
            expand,
            pet_items,
            size_items,
            bubbles,
            aura,
            shadows,
            click_through,
            hide_pet,
            start_at_login,
        }
    }

    /// Refreshes texts and check marks in place — valid only while `same_menu_structure`
    /// holds between the model this menu was built from and `model`.
    pub fn apply(&self, model: &MenuModel) {
        self.header.set_text(&model.header);
        if let (Some(item), Some(usage_line)) = (&self.usage, &model.usage_line) {
            item.set_text(usage_line);
        }
        if let Some(automatic) = &self.automatic {
            automatic.set_checked(model.is_following_automatically);
        }
        for (item, row) in self.session_rows.iter().zip(&model.sessions) {
            item.set_text(&row.title);
            item.set_checked(row.is_pinned);
        }
        for (metric, item) in &self.gauge_items {
            item.set_checked(*metric == model.gauge_metric);
        }
        self.expand.set_checked(model.always_expanded);
        for (item, (_, _, is_selected)) in self.pet_items.iter().zip(&model.pets) {
            item.set_checked(*is_selected);
        }
        let nearest = nearest_size_option(model.pixel_scale);
        for (scale, item) in &self.size_items {
            item.set_checked(*scale == nearest);
        }
        self.bubbles.set_checked(model.bubbles_hidden);
        self.aura.set_checked(model.effort_aura_hidden);
        self.shadows.set_checked(model.agent_shadows_hidden);
        self.click_through.set_checked(model.click_through);
        self.hide_pet.set_text(if model.pet_hidden { "Show pet" } else { "Hide pet" });
        self.start_at_login.set_checked(model.start_at_login);
    }
}

/// One-shot build for the right-click context menu, which is rebuilt fresh every time it
/// opens and so has no use for the handles.
pub fn build_menu(model: &MenuModel) -> Menu {
    TrayMenu::build(model).menu
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

    fn model_fixture() -> MenuModel {
        MenuModel {
            header: "Boo · proj: Working".to_string(),
            usage_line: Some("ctx 60% left".to_string()),
            sessions: vec![
                SessionMenuRow { session_id: "s1".into(), title: "proj — Working".into(), is_pinned: false },
                SessionMenuRow { session_id: "s2".into(), title: "other — Done".into(), is_pinned: true },
            ],
            is_following_automatically: false,
            pets: vec![("boo-ghost".into(), "Boo".into(), true)],
            pixel_scale: 5.0,
            gauge_metric: GaugeMetric::ContextRemaining,
            always_expanded: false,
            bubbles_hidden: false,
            click_through: false,
            pet_hidden: false,
            effort_aura_hidden: false,
            agent_shadows_hidden: false,
            start_at_login: true,
        }
    }

    #[test]
    fn text_and_check_changes_keep_the_menu_structure() {
        // Everything that changes on a busy tick must be updatable in place: replacing
        // the menu for these is what killed clicks while it was open.
        let base = model_fixture();
        let mut ticked = base.clone();
        ticked.header = "Boo · proj: Thinking".to_string();
        ticked.usage_line = Some("ctx 59% left".to_string());
        ticked.sessions[0].title = "proj — Thinking".to_string();
        ticked.sessions[1].is_pinned = false;
        ticked.is_following_automatically = true;
        ticked.pixel_scale = 7.0;
        ticked.gauge_metric = GaugeMetric::Off;
        ticked.pet_hidden = true;
        ticked.start_at_login = false;
        assert!(same_menu_structure(&base, &ticked));
    }

    #[test]
    fn row_changes_break_the_menu_structure() {
        let base = model_fixture();

        let mut session_gone = base.clone();
        session_gone.sessions.pop();
        assert!(!same_menu_structure(&base, &session_gone));

        let mut session_swapped = base.clone();
        session_swapped.sessions[0].session_id = "s9".into();
        assert!(!same_menu_structure(&base, &session_swapped));

        let mut usage_gone = base.clone();
        usage_gone.usage_line = None;
        assert!(!same_menu_structure(&base, &usage_gone));

        let mut pet_added = base.clone();
        pet_added.pets.push(("mochi-cat".into(), "Mochi".into(), false));
        assert!(!same_menu_structure(&base, &pet_added));

        let mut pet_renamed = base.clone();
        pet_renamed.pets[0].1 = "Boo  (custom)".into();
        assert!(!same_menu_structure(&base, &pet_renamed));
    }

    #[test]
    fn menu_ids_round_trip() {
        let actions = [
            MenuAction::PinSession("abc-123".to_string()),
            MenuAction::FollowSessionsAutomatically,
            MenuAction::InstallStatusLine,
            MenuAction::ToggleAlwaysExpanded,
            MenuAction::OpenPetsFolder,
            MenuAction::InstallHooks,
            MenuAction::InstallMcp,
            MenuAction::ToggleStartAtLogin,
            MenuAction::OpenHookLog,
            MenuAction::SelectPet("airou-felyne".to_string()),
            MenuAction::SelectSize(3.0),
            MenuAction::SelectSize(7.0),
            MenuAction::SelectGauge(GaugeMetric::FiveHourRemaining),
            MenuAction::SelectGauge(GaugeMetric::Off),
            MenuAction::ToggleBubbles,
            MenuAction::ToggleEffortAura,
            MenuAction::ToggleAgentShadows,
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
