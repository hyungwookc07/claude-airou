//! claude-airou — Rust build. Command dispatch lives in `cli.rs`; on-disk formats are shared
//! with the Swift app (see ../../Sources/ClaudeAirou). The overlay itself is macOS-only for
//! now; every other subcommand is fully cross-platform.

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
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(cli::dispatch(arguments));
}
