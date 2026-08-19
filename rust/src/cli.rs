//! Hand-rolled CLI mirroring the Swift `CommandLineInterface`: same commands, same flags,
//! so scripts and the Makefile work against either binary.

use std::collections::{HashMap, HashSet};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub const USAGE_TEXT: &str = r#"claude-airou — a Codex-style desktop pet for Claude Code (Rust build)

USAGE
  claude-airou                       Run the overlay (menu bar icon + floating pet; macOS)
  claude-airou run                   Same as above
  claude-airou setup [--minimal] [--no-autostart] [--with-statusline] [--with-mcp]
                                   Wire everything up in one go: Claude Code hooks, the
                                   /hatch-pet skill and start-at-login (--minimal keeps
                                   hooks + skill only). The status line and the MCP server
                                   rewrite config you probably own, so they are opt-in.
  claude-airou uninstall             Undo setup (hooks, status line, MCP, skill, login item);
                                   keeps ~/.claude-airou and the binary itself
  claude-airou hook                  Claude Code hook entry point (reads hook JSON on stdin)
  claude-airou install-hooks [--print] [--settings PATH]
                                   Merge hook entries into ~/.claude/settings.json (backup first)
                                   --print only prints the JSON snippet, changes nothing
  claude-airou uninstall-hooks [--settings PATH]
  claude-airou install-statusline [--settings PATH]
                                   Feed the usage gauge from the Claude Code status line; your
                                   existing status line keeps running (passthrough)
  claude-airou uninstall-statusline  Restore the original status line
  claude-airou statusline            Status line entry point (reads the status line JSON on stdin)
  claude-airou mcp                   MCP server over stdio, so Claude chat (the Claude
                                   desktop app) can drive the pet (pet_status, hatch_pet, …)
  claude-airou install-mcp [--print] [--config PATH]
                                   Register the MCP server in the Claude desktop app
                                   (backup first); restart the app afterwards
  claude-airou uninstall-mcp [--config PATH]
  claude-airou simulate STATE [--message TEXT] [--session ID] [--cwd PATH]
                                   Write a fake session so you can see the pet react
                                   STATE: hello | idle | thinking | working | waiting_approval |
                                          needs_input | done | error | clear | demo
  claude-airou pets                  List available pets (built-in + ~/.claude-airou/pets)
  claude-airou validate FILE.json    Validate a pet JSON file
  claude-airou render PET_ID|FILE [--out DIR] [--scale N] [--bg #RRGGBB]
                                   Render every frame to PNG (+ sheet.png) to eyeball pixel art
  claude-airou preview PET_ID|FILE [--state STATE] [--solid]
                                   Print frames as ASCII
  claude-airou status                Print the sessions the overlay currently sees
  claude-airou snapshot [--out FILE.png]
                                   Ask the running overlay to save a PNG of itself
  claude-airou click [primary|X]     Click the running overlay (primary pet, or x in points) — for testing
  claude-airou help

FILES
  ~/.claude-airou/config.json        preferences (pet, size, position)
  ~/.claude-airou/pets/*.json        your custom pets (see skills/hatch-pet)
  ~/.claude-airou/state/*.json       live session state written by the hook and the MCP server
  ~/.claude-airou/hook.log           what the hook saw (auto-truncated)
  ~/.claude-airou/mcp.log            what the MCP server saw (auto-truncated)
"#;

#[derive(Debug, Default, Clone)]
pub struct Parsed {
    pub positional: Vec<String>,
    pub options: HashMap<String, String>,
    pub flags: HashSet<String>,
}

impl Parsed {
    pub fn option(&self, name: &str) -> Option<&str> {
        self.options.get(name).map(String::as_str)
    }

    pub fn has_flag(&self, name: &str) -> bool {
        self.flags.contains(name)
    }
}

/// Options that never take a value; everything else written as `--name value` is an option.
const BOOLEAN_FLAG_NAMES: [&str; 10] = [
    "solid",
    "print",
    "help",
    "version",
    "h",
    "yes",
    "minimal",
    "no-autostart",
    "with-statusline",
    "with-mcp",
];

/// `--key value` becomes an option, `--flag` becomes a flag; `--key=value` is also accepted.
pub fn parse(arguments: &[String]) -> Parsed {
    let mut parsed = Parsed::default();
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        if argument == "-h" {
            parsed.flags.insert("help".to_string());
        } else if let Some(name) = argument.strip_prefix("--") {
            if let Some((key, value)) = name.split_once('=') {
                parsed.options.insert(key.to_string(), value.to_string());
            } else if BOOLEAN_FLAG_NAMES.contains(&name) {
                parsed.flags.insert(name.to_string());
            } else if index + 1 < arguments.len() && !arguments[index + 1].starts_with("--") {
                parsed.options.insert(name.to_string(), arguments[index + 1].clone());
                index += 1;
            } else {
                parsed.flags.insert(name.to_string());
            }
        } else {
            parsed.positional.push(argument.clone());
        }
        index += 1;
    }
    parsed
}

/// Returns the process exit code.
pub fn dispatch(arguments: Vec<String>) -> i32 {
    let parsed = parse(&arguments);
    if parsed.has_flag("help") || parsed.has_flag("h") {
        println!("{USAGE_TEXT}");
        return 0;
    }
    if parsed.has_flag("version") {
        println!("claude-airou {VERSION} (rust)");
        return 0;
    }
    let command = parsed.positional.first().cloned().unwrap_or_else(|| "run".to_string());
    let rest: Vec<String> = parsed.positional.iter().skip(1).cloned().collect();

    match command.as_str() {
        "run" => run_overlay(),
        "hook" => crate::hook::run(),
        "statusline" => crate::statusline::run(&arguments),
        "mcp" => crate::mcp::run(),
        "setup" => crate::setup::run_setup(&parsed),
        "uninstall" => crate::setup::run_uninstall(&parsed),
        "install-hooks" => crate::install::run_install_hooks(&parsed),
        "uninstall-hooks" => crate::install::run_uninstall_hooks(&parsed),
        "install-statusline" => crate::install::run_install_statusline(&parsed),
        "uninstall-statusline" => crate::install::run_uninstall_statusline(&parsed),
        "install-mcp" => crate::install::run_install_mcp(&parsed),
        "uninstall-mcp" => crate::install::run_uninstall_mcp(&parsed),
        "simulate" => crate::cli_commands::run_simulate(&rest, &parsed),
        "pets" | "list" => crate::cli_commands::run_list_pets(),
        "validate" => crate::cli_commands::run_validate(&rest),
        "render" => crate::cli_commands::run_render(&rest, &parsed),
        "preview" => crate::cli_commands::run_preview(&rest, &parsed),
        "status" | "sessions" => crate::cli_commands::run_status(),
        "snapshot" => crate::cli_commands::run_snapshot(&parsed),
        "click" => crate::cli_commands::run_click(&rest),
        "help" => {
            println!("{USAGE_TEXT}");
            0
        }
        "version" => {
            println!("claude-airou {VERSION} (rust)");
            0
        }
        other => {
            crate::logging::eprint_line(&format!("claude-airou: unknown command \"{other}\"\n"));
            crate::logging::eprint_line(USAGE_TEXT);
            2
        }
    }
}

#[cfg(target_os = "macos")]
fn run_overlay() -> i32 {
    crate::overlay::run()
}

#[cfg(not(target_os = "macos"))]
fn run_overlay() -> i32 {
    crate::logging::eprint_line(
        "claude-airou: the overlay is macOS-only in this build (hook / mcp / CLI commands all work here).\n\
         Windows and Linux overlays are on the roadmap — the state files this build writes are already portable.",
    );
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_options_flags_positional() {
        let parsed = parse(&args(&[
            "simulate",
            "working",
            "--message",
            "hi there",
            "--solid",
            "--scale=10",
        ]));
        assert_eq!(parsed.positional, vec!["simulate", "working"]);
        assert_eq!(parsed.option("message"), Some("hi there"));
        assert_eq!(parsed.option("scale"), Some("10"));
        assert!(parsed.has_flag("solid"));
    }

    #[test]
    fn trailing_option_without_value_becomes_flag() {
        let parsed = parse(&args(&["render", "mochi-cat", "--out"]));
        assert!(parsed.has_flag("out"));
    }
}
