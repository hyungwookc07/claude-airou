//! Installers: merge/remove our entries in `~/.claude/settings.json` (hooks + statusLine)
//! and the Claude desktop app's `claude_desktop_config.json` (MCP server). Ports of
//! `Install/HooksInstaller.swift`, `Install/StatusLineInstaller.swift` and
//! `Install/MCPInstaller.swift` — same manners everywhere:
//!
//! - Only our own keys are touched; foreign entries and unknown shapes are left alone
//!   (a non-object `hooks`/`mcpServers` refuses with a readable error instead of clobbering).
//! - A timestamped backup (`<file>.claude-airou-backup-YYYYmmdd-HHMMSS[-N]`) is written
//!   before any change; no change → no backup, file left byte-for-byte alone.
//! - Idempotent: re-running updates the command path instead of duplicating entries.
//! - Hook command is the shell-single-quoted absolute path of the current executable +
//!   " hook" with timeout 10; entries are recognised by the "claude-airou"/"claude-pet"
//!   markers or by resolving the first word to our own binary (see `isOurHandler`).
//! - Status line install stashes the previous `statusLine` value in
//!   `paths::statusline_passthrough_file()`; uninstall restores it.
//! - MCP entry: `mcpServers["claude-airou"] = {"command": <exe>, "args": ["mcp"]}` (exec
//!   form, no shell). Uninstall removes the key and drops `mcpServers` when empty.
//! - `--print` prints the would-be JSON snippet and changes nothing.
//!
//! Each `run_*` prints the same summary lines as the Swift CLI and returns the exit code.

use crate::cli::Parsed;

pub fn run_install_hooks(parsed: &Parsed) -> i32 {
    let _ = parsed;
    todo!("port HooksInstaller.install + CLI wrapper")
}

pub fn run_uninstall_hooks(parsed: &Parsed) -> i32 {
    let _ = parsed;
    todo!("port HooksInstaller.uninstall + CLI wrapper")
}

pub fn run_install_statusline(parsed: &Parsed) -> i32 {
    let _ = parsed;
    todo!("port StatusLineInstaller.install + CLI wrapper")
}

pub fn run_uninstall_statusline(parsed: &Parsed) -> i32 {
    let _ = parsed;
    todo!("port StatusLineInstaller.uninstall + CLI wrapper")
}

pub fn run_install_mcp(parsed: &Parsed) -> i32 {
    let _ = parsed;
    todo!("port MCPInstaller.install + CLI wrapper")
}

pub fn run_uninstall_mcp(parsed: &Parsed) -> i32 {
    let _ = parsed;
    todo!("port MCPInstaller.uninstall + CLI wrapper")
}
