//! Subcommand implementations: simulate / pets / validate / render / preview / status.
//! Ports of the matching `runX` functions in `CLI/CommandLineInterface.swift` — same
//! output shapes (tab-separated status lines, "OK: id (Name the species), grid WxH, N
//! state(s)", demo script timings, default messages per state, etc.).

use crate::cli::Parsed;

/// `simulate STATE|clear|demo [--message TEXT] [--session ID] [--cwd PATH]`.
pub fn run_simulate(positional: &[String], parsed: &Parsed) -> i32 {
    let _ = (positional, parsed);
    todo!("port runSimulate (incl. the demo script and defaultMessage(for:))")
}

/// Lists pets: "id\tName the species\tWxH\t<origin>" (+ "skipped: …" to stderr).
pub fn run_list_pets() -> i32 {
    todo!("port runListPets")
}

pub fn run_validate(positional: &[String]) -> i32 {
    let _ = positional;
    todo!("port runValidate (OK line + warnings, INVALID to stderr, exit 1/2)")
}

/// `render PET_ID|FILE [--out DIR] [--scale N] [--bg #RRGGBB]` — scale 1–64 (default 8).
pub fn run_render(positional: &[String], parsed: &Parsed) -> i32 {
    let _ = (positional, parsed);
    todo!("port runRender")
}

/// `preview PET_ID|FILE [--state STATE] [--solid]` — "== state [i] ==" + ascii frames.
pub fn run_preview(positional: &[String], parsed: &Parsed) -> i32 {
    let _ = (positional, parsed);
    todo!("port runPreview")
}

/// Tab-separated session dump incl. effective state, age and usage summary
/// ("ctx 42% 5h 10% 7d 3% status_line").
pub fn run_status() -> i32 {
    todo!("port runStatus")
}
