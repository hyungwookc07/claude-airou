//! All on-disk locations used by claude-airou. Everything lives under `~/.claude-airou`
//! (override with `CLAUDE_AIROU_HOME`). Byte-compatible with the Swift app's `AppPaths`,
//! so both implementations can run against the same directory.

use std::path::PathBuf;

pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

pub fn root_dir() -> PathBuf {
    if let Ok(overridden) = std::env::var("CLAUDE_AIROU_HOME") {
        if !overridden.is_empty() {
            return expand_tilde(&overridden);
        }
    }
    home_dir().join(".claude-airou")
}

pub fn state_dir() -> PathBuf {
    root_dir().join("state")
}

pub fn pets_dir() -> PathBuf {
    root_dir().join("pets")
}

pub fn config_file() -> PathBuf {
    root_dir().join("config.json")
}

pub fn hook_log_file() -> PathBuf {
    root_dir().join("hook.log")
}

pub fn mcp_log_file() -> PathBuf {
    root_dir().join("mcp.log")
}

pub fn statusline_passthrough_file() -> PathBuf {
    root_dir().join("statusline-passthrough.json")
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // overlay-only today
pub fn overlay_lock_file() -> PathBuf {
    root_dir().join("overlay.lock")
}

/// `claude-airou snapshot` drops this file; the running overlay answers by writing
/// `snapshot_image_file()` (same names as Swift's `AppPaths`).
pub fn snapshot_request_file() -> PathBuf {
    root_dir().join("snapshot.request")
}

pub fn snapshot_image_file() -> PathBuf {
    root_dir().join("snapshot.png")
}

/// `claude-airou click` writes the click target here; the overlay consumes it.
pub fn click_request_file() -> PathBuf {
    root_dir().join("click.request")
}

/// `~/.claude/settings.json` — where Claude Code reads hooks and the status line from.
pub fn claude_settings_file() -> PathBuf {
    home_dir().join(".claude").join("settings.json")
}

/// `~/.claude/skills/hatch-pet` — where `claude-airou setup` writes the /hatch-pet skill.
pub fn claude_hatch_pet_skill_dir() -> PathBuf {
    home_dir().join(".claude").join("skills").join("hatch-pet")
}

/// The LaunchAgent that starts the overlay at login (written by `claude-airou setup`).
pub fn overlay_launch_agent_file() -> PathBuf {
    home_dir()
        .join("Library/LaunchAgents")
        .join(format!("{OVERLAY_LAUNCH_AGENT_LABEL}.plist"))
}

/// Pre-rename LaunchAgent (`claude-pet`), removed by setup/uninstall when found.
pub fn legacy_overlay_launch_agent_file() -> PathBuf {
    home_dir()
        .join("Library/LaunchAgents")
        .join(format!("{LEGACY_OVERLAY_LAUNCH_AGENT_LABEL}.plist"))
}

pub const OVERLAY_LAUNCH_AGENT_LABEL: &str = "dev.claude-airou.overlay";
pub const LEGACY_OVERLAY_LAUNCH_AGENT_LABEL: &str = "dev.claude-pet.overlay";

/// Where the Claude desktop app (chat) reads its MCP servers from.
#[cfg(target_os = "macos")]
pub fn claude_desktop_config_file() -> PathBuf {
    home_dir().join("Library/Application Support/Claude/claude_desktop_config.json")
}

#[cfg(target_os = "windows")]
pub fn claude_desktop_config_file() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| home_dir().join("AppData/Roaming"))
        .join("Claude/claude_desktop_config.json")
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn claude_desktop_config_file() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| home_dir().join(".config"))
        .join("Claude/claude_desktop_config.json")
}

/// Known deviation from Swift's `expandingTildeInPath`: the `~otheruser/…` form is not
/// expanded (needs a passwd lookup); only `~` and `~/…` are. Nobody sets that form in
/// practice, and both binaries agree on the common cases.
pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        return home_dir().join(rest);
    }
    if path == "~" {
        return home_dir();
    }
    PathBuf::from(path)
}

// MARK: - Legacy (the project was called claude-pet before it became claude-airou)

/// Moves `~/.claude-pet` to `~/.claude-airou` once, so config, pets, state and the
/// status-line passthrough survive the rename. No-op when a home override is set or the
/// new dir exists. Port of Swift's `AppPaths.migrateLegacyDirectoryIfNeeded`, run before
/// every command (see main.rs).
pub fn migrate_legacy_dir_if_needed() {
    if std::env::var_os("CLAUDE_AIROU_HOME").is_some() {
        return;
    }
    migrate_legacy_dir(&home_dir().join(".claude-pet"), &home_dir().join(".claude-airou"));
}

/// Testable core: rename `legacy` onto `new` when only `legacy` exists; the overlay lock
/// and per-session state are transient (hooks refill state), so the lock is dropped.
fn migrate_legacy_dir(legacy: &std::path::Path, new: &std::path::Path) {
    if new.exists() || !legacy.exists() {
        return;
    }
    match std::fs::rename(legacy, new) {
        Ok(()) => {
            let _ = std::fs::remove_file(new.join("overlay.lock"));
        }
        Err(error) => crate::logging::eprint_line(&format!(
            "claude-airou: could not migrate {} → {}: {error}",
            legacy.display(),
            new.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn migrates_only_when_new_dir_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join(".claude-pet");
        let new = dir.path().join(".claude-airou");
        std::fs::create_dir_all(legacy.join("pets")).unwrap();
        std::fs::write(legacy.join("config.json"), b"{}").unwrap();
        std::fs::write(legacy.join("overlay.lock"), b"123").unwrap();

        super::migrate_legacy_dir(&legacy, &new);
        assert!(!legacy.exists());
        assert!(new.join("config.json").exists());
        assert!(new.join("pets").exists());
        assert!(!new.join("overlay.lock").exists(), "transient lock is dropped");

        // Second run (legacy gone) and a run with both dirs present are no-ops.
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("other.json"), b"{}").unwrap();
        super::migrate_legacy_dir(&legacy, &new);
        assert!(legacy.join("other.json").exists(), "never overwrites an existing new dir");
    }
}

pub fn ensure_dir(path: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)
}
