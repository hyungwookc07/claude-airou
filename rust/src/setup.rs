//! `claude-airou setup` / `claude-airou uninstall` — the one command a freshly downloaded
//! binary needs. Everything the Makefile used to do target by target (hooks, the /hatch-pet
//! skill, the login LaunchAgent) happens here instead, so an install script that only drops
//! a binary in `~/.local/bin` can finish the job without a repo checkout.
//!
//! Defaults: hooks + the /hatch-pet skill, and nothing else. Everything that changes the
//! machine beyond that — start-at-login, the status line, the MCP server — is a switch in
//! the tray menu (or an explicit flag here), because a `curl … | sh` install cannot ask and
//! a login item nobody agreed to is not a good surprise.

use crate::cli::Parsed;
#[cfg(target_os = "macos")] // only the LaunchAgent side takes paths directly
use std::path::Path;

/// Baked into the binary so `setup` can install the skill without the repo (the Makefile
/// copies the very same file from `skills/hatch-pet/`).
const HATCH_PET_SKILL_MARKDOWN: &str = include_str!("../../skills/hatch-pet/SKILL.md");

const SKILL_FILE_NAME: &str = "SKILL.md";

/// One line per step, so the install script's output stays scannable.
fn report_step(step_name: &str, outcome: Result<String, String>, failure_count: &mut usize) {
    match outcome {
        Ok(summary) => {
            println!("  ✓ {step_name}");
            for line in summary.lines().filter(|line| !line.trim().is_empty()) {
                println!("      {line}");
            }
        }
        Err(message) => {
            *failure_count += 1;
            println!("  ✗ {step_name}");
            for line in message.lines() {
                println!("      {line}");
            }
        }
    }
}

pub fn run_setup(parsed: &Parsed) -> i32 {
    // --minimal and --no-autostart are the old spelling of the default; still accepted.
    let should_install_autostart = parsed.has_flag("with-autostart") && !parsed.has_flag("no-autostart");
    let should_install_statusline = parsed.has_flag("with-statusline");
    let should_install_mcp = parsed.has_flag("with-mcp");

    println!("Setting up claude-airou…");
    let mut failure_count = 0;

    report_step(
        "Claude Code hooks (~/.claude/settings.json)",
        crate::install::install_hooks_at_default_paths(),
        &mut failure_count,
    );
    report_step("/hatch-pet skill", install_hatch_pet_skill(), &mut failure_count);

    if should_install_statusline {
        report_step(
            "status line (usage gauge)",
            crate::install::install_statusline_at_default_paths(),
            &mut failure_count,
        );
    }
    if should_install_mcp {
        report_step(
            "MCP server (Claude desktop app)",
            crate::install::install_mcp_at_default_paths(),
            &mut failure_count,
        );
    }
    if should_install_autostart {
        report_step("start at login", install_login_autostart(), &mut failure_count);
    }

    println!();
    if failure_count > 0 {
        crate::logging::eprint_line(&format!(
            "claude-airou: setup finished with {failure_count} failed step(s) — see above."
        ));
        return 1;
    }

    println!("Done. Next:");
    println!("  • Start a NEW Claude Code session — sessions that are already open do not re-read the hook settings.");
    if should_install_autostart || is_login_autostart_installed() {
        println!("  • The overlay is running now and will start again at login (menu bar 🐾).");
        println!("  • The rest is in the menu bar 🐾 menu: the MCP server and the gauge's status line feed.");
    } else {
        println!("  • Everything else is in the menu bar 🐾 menu: \"Start at login\" (off for now, so the");
        println!("    pet will not come back by itself after a reboot), the MCP server, and the gauge's");
        println!("    status line feed.");
    }
    println!("  • Undo everything with `claude-airou uninstall`.");
    0
}

pub fn run_uninstall(_parsed: &Parsed) -> i32 {
    println!("Removing claude-airou integrations…");
    let mut failure_count = 0;

    report_step(
        "Claude Code hooks",
        crate::install::uninstall_hooks_at_default_paths(),
        &mut failure_count,
    );
    report_step(
        "status line",
        crate::install::uninstall_statusline_at_default_paths(),
        &mut failure_count,
    );
    report_step(
        "MCP server",
        crate::install::uninstall_mcp_at_default_paths(),
        &mut failure_count,
    );
    report_step("/hatch-pet skill", remove_hatch_pet_skill(), &mut failure_count);
    report_step("start at login", remove_login_autostart(), &mut failure_count);

    println!();
    println!("Your pets, config and state are untouched (~/.claude-airou — delete it by hand if you want them gone).");
    match std::env::current_exe() {
        Ok(path) => println!("The binary itself is still at {} — remove it to finish.", path.display()),
        Err(_) => println!("Remove the claude-airou binary to finish."),
    }
    if failure_count > 0 {
        crate::logging::eprint_line(&format!(
            "claude-airou: uninstall finished with {failure_count} failed step(s) — see above."
        ));
        return 1;
    }
    0
}

// MARK: - /hatch-pet skill

fn install_hatch_pet_skill() -> Result<String, String> {
    let skill_dir = crate::paths::claude_hatch_pet_skill_dir();
    std::fs::create_dir_all(&skill_dir)
        .map_err(|error| format!("could not create {}: {error}", skill_dir.display()))?;
    let skill_file = skill_dir.join(SKILL_FILE_NAME);
    crate::state_store::write_atomic(&skill_file, HATCH_PET_SKILL_MARKDOWN.as_bytes())
        .map_err(|error| format!("could not write {}: {error}", skill_file.display()))?;
    Ok(format!("wrote {}", skill_file.display()))
}

fn remove_hatch_pet_skill() -> Result<String, String> {
    let skill_dir = crate::paths::claude_hatch_pet_skill_dir();
    if !skill_dir.exists() {
        return Ok("not installed".to_string());
    }
    std::fs::remove_dir_all(&skill_dir)
        .map_err(|error| format!("could not remove {}: {error}", skill_dir.display()))?;
    Ok(format!("removed {}", skill_dir.display()))
}

// MARK: - Tray menu entry points

/// True when the login item is registered (the tray shows a check mark for it).
#[cfg(target_os = "macos")]
pub fn is_login_autostart_installed() -> bool {
    crate::paths::overlay_launch_agent_file().exists()
}

/// No login item exists off macOS yet, so the tray check mark is always off there.
#[cfg(not(target_os = "macos"))]
#[allow(dead_code)]
pub fn is_login_autostart_installed() -> bool {
    false
}

/// Turns the login item on; Ok carries the summary the tray shows in an alert.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn install_login_autostart_at_default_paths() -> Result<String, String> {
    install_login_autostart()
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn remove_login_autostart_at_default_paths() -> Result<String, String> {
    remove_login_autostart()
}

// MARK: - Start at login (LaunchAgent)

#[cfg(target_os = "macos")]
fn install_login_autostart() -> Result<String, String> {
    let executable = current_executable_path()?;
    let plist_path = crate::paths::overlay_launch_agent_file();
    let plist_dir = plist_path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", plist_path.display()))?;
    std::fs::create_dir_all(plist_dir)
        .map_err(|error| format!("could not create {}: {error}", plist_dir.display()))?;
    crate::state_store::write_atomic(
        &plist_path,
        launch_agent_plist(&executable, crate::paths::OVERLAY_LAUNCH_AGENT_LABEL).as_bytes(),
    )
    .map_err(|error| format!("could not write {}: {error}", plist_path.display()))?;

    // A previous version (and the pre-rename claude-pet agent) would keep the old binary
    // alive, so unload both before loading this one. RunAtLoad starts the overlay right away.
    let legacy_plist_path = crate::paths::legacy_overlay_launch_agent_file();
    if legacy_plist_path.exists() {
        let _ = run_launchctl(&["bootout", &gui_domain_target()?], Some(&legacy_plist_path));
        let _ = std::fs::remove_file(&legacy_plist_path);
    }
    let _ = run_launchctl(&["bootout", &gui_domain_target()?], Some(&plist_path));
    run_launchctl(&["bootstrap", &gui_domain_target()?], Some(&plist_path))?;
    Ok(format!("{} (overlay started)", plist_path.display()))
}

#[cfg(target_os = "macos")]
fn remove_login_autostart() -> Result<String, String> {
    let plist_path = crate::paths::overlay_launch_agent_file();
    let legacy_plist_path = crate::paths::legacy_overlay_launch_agent_file();
    if !plist_path.exists() && !legacy_plist_path.exists() {
        return Ok("not installed".to_string());
    }
    for path in [&plist_path, &legacy_plist_path] {
        if path.exists() {
            let _ = run_launchctl(&["bootout", &gui_domain_target()?], Some(path));
            std::fs::remove_file(path)
                .map_err(|error| format!("could not remove {}: {error}", path.display()))?;
        }
    }
    Ok(format!("removed {}", plist_path.display()))
}

#[cfg(not(target_os = "macos"))]
fn install_login_autostart() -> Result<String, String> {
    Err("start at login is macOS-only for now.".to_string())
}

#[cfg(not(target_os = "macos"))]
fn remove_login_autostart() -> Result<String, String> {
    Ok("not installed".to_string())
}

#[cfg(target_os = "macos")]
fn current_executable_path() -> Result<std::path::PathBuf, String> {
    std::env::current_exe().map_err(|error| format!("could not locate this binary: {error}"))
}

/// `gui/<uid>` — the launchd domain a login-session agent belongs to.
#[cfg(target_os = "macos")]
fn gui_domain_target() -> Result<String, String> {
    let output = std::process::Command::new("id")
        .arg("-u")
        .output()
        .map_err(|error| format!("could not run `id -u`: {error}"))?;
    let user_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if user_id.is_empty() {
        return Err("could not read the current user id from `id -u`.".to_string());
    }
    Ok(format!("gui/{user_id}"))
}

#[cfg(target_os = "macos")]
fn run_launchctl(arguments: &[&str], plist_path: Option<&Path>) -> Result<(), String> {
    let mut command = std::process::Command::new("launchctl");
    command.args(arguments);
    if let Some(path) = plist_path {
        command.arg(path);
    }
    let output = command
        .output()
        .map_err(|error| format!("could not run launchctl: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(format!("launchctl {} failed: {stderr}", arguments.join(" ")))
}

/// Minimal XML escaping — the executable path is the only interpolated value, but a home
/// directory with `&` in it would otherwise produce a plist launchd refuses to parse.
#[cfg(target_os = "macos")]
fn xml_escaped(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(target_os = "macos")]
fn launch_agent_plist(executable: &Path, label: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>{label}</string>
  <key>ProgramArguments</key><array><string>{path}</string><string>run</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><false/>
</dict></plist>
"#,
        label = xml_escaped(label),
        path = xml_escaped(&executable.to_string_lossy()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_markdown_is_baked_in() {
        assert!(
            HATCH_PET_SKILL_MARKDOWN.contains("hatch-pet"),
            "the embedded skill should be the real SKILL.md"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn plist_escapes_the_executable_path() {
        let plist = launch_agent_plist(
            Path::new("/Users/a&b/.local/bin/claude-airou"),
            "dev.claude-airou.overlay",
        );
        assert!(plist.contains("<string>/Users/a&amp;b/.local/bin/claude-airou</string>"));
        assert!(plist.contains("<string>run</string>"));
        assert!(plist.contains("<key>RunAtLoad</key><true/>"));
        assert!(!plist.contains("a&b"), "raw ampersands would break plist parsing");
    }
}
