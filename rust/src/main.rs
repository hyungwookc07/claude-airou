//! claude-airou. Command dispatch lives in `cli.rs`; on-disk formats are unchanged from the
//! original Swift app (removed in v1.0 — see git history, commit 3037817). The overlay itself
//! is macOS-only for now; every other subcommand is fully cross-platform.

mod cli;
mod cli_commands;
mod hook;
mod hook_mapper;
mod install;
mod logging;
mod mcp;
mod mcp_tools;
mod model;
mod paths;
mod pets;
mod render;
mod state_store;
mod statusline;

#[cfg(target_os = "macos")]
mod overlay;

fn main() {
    // Like Swift's main: adopt a pre-rename ~/.claude-pet directory before any command runs.
    paths::migrate_legacy_dir_if_needed();
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(cli::dispatch(arguments));
}
