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

/// `~/.claude/settings.json` — where Claude Code reads hooks and the status line from.
pub fn claude_settings_file() -> PathBuf {
    home_dir().join(".claude").join("settings.json")
}

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

pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        return home_dir().join(rest);
    }
    if path == "~" {
        return home_dir();
    }
    PathBuf::from(path)
}

pub fn ensure_dir(path: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)
}
