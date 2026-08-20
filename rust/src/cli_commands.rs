//! Subcommand implementations: simulate / pets / validate / render / preview / status.
//! Ports of the matching `runX` functions in `CLI/CommandLineInterface.swift` — same
//! output shapes (tab-separated status lines, "OK: id (Name the species), grid WxH, N
//! state(s)", demo script timings, default messages per state, etc.).

use crate::cli::Parsed;
use crate::logging::eprint_line;
use crate::model::{now_epoch_secs, PetState, SessionSnapshot, SessionUsageSnapshot};
use crate::pets::{PetDefinition, PetLibrary, PixelColor};
use crate::state_store::StateStore;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The 10-step `simulate demo` script (state, message, sleep seconds).
const DEMO_SCRIPT: [(PetState, &str, f64); 10] = [
    (PetState::Hello, "Hi! Ready when you are", 3.0),
    (PetState::Thinking, "Thinking…", 3.0),
    (PetState::Working, "Reading main.rs", 3.0),
    (PetState::Working, "Running: cargo build", 3.0),
    (PetState::WaitingApproval, "Approve? Running: git push", 5.0),
    (PetState::Working, "Editing README.md", 3.0),
    (PetState::Error, "Bash failed — recovering…", 3.0),
    (PetState::Thinking, "Thinking…", 2.0),
    (PetState::Done, "Done!", 4.0),
    (PetState::NeedsInput, "Waiting for you…", 4.0),
];

/// Collected stdout/stderr lines plus the exit code, so tests can assert exact output.
struct CommandOutput {
    out: Vec<String>,
    err: Vec<String>,
    code: i32,
}

fn err_out(message: String, code: i32) -> CommandOutput {
    CommandOutput { out: Vec::new(), err: vec![message], code }
}

fn emit(output: &CommandOutput) -> i32 {
    for line in &output.out {
        println!("{line}");
    }
    for line in &output.err {
        eprint_line(line);
    }
    output.code
}

fn state_list_text() -> String {
    PetState::ALL.iter().map(|state| state.raw()).collect::<Vec<_>>().join(" | ")
}

/// `simulate STATE|clear|demo [--message TEXT] [--session ID] [--cwd PATH]`.
pub fn run_simulate(positional: &[String], parsed: &Parsed) -> i32 {
    run_simulate_impl(positional, parsed, &|seconds| {
        std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
    })
}

/// The demo prints and writes live between sleeps, so output stays interactive; the
/// sleeper is injected only so tests can run the script without the 33s wait.
fn run_simulate_impl(positional: &[String], parsed: &Parsed, sleep: &dyn Fn(f64)) -> i32 {
    let Some(state_text) = positional.first() else {
        eprint_line(&format!(
            "claude-airou simulate: missing STATE ({} | clear | demo)",
            state_list_text()
        ));
        return 2;
    };
    let store = StateStore::default();
    let session_id = parsed.option("session").unwrap_or("simulated").to_string();
    let cwd = parsed
        .option("cwd")
        .map(str::to_string)
        .unwrap_or_else(current_dir_string);

    if state_text == "clear" {
        store.remove(&session_id);
        println!("Removed simulated session \"{session_id}\".");
        return 0;
    }

    let write = |state: PetState, message: &str| {
        let snapshot = SessionSnapshot {
            session_id: session_id.clone(),
            cwd: cwd.clone(),
            state,
            message: message.to_string(),
            last_event_name: "simulate".to_string(),
            tool_name: None,
            updated_at_epoch_seconds: now_epoch_secs(),
            pending_tool_use_id: None,
            active_agent_ids: Vec::new(),
        };
        match store.write(&snapshot) {
            Ok(()) => println!("{}: {message}", state.raw()),
            Err(error) => eprint_line(&format!("claude-airou simulate: {error}")),
        }
    };

    if state_text == "demo" {
        println!("Demo: cycling through states (Ctrl-C to stop). Session \"{session_id}\".");
        for (state, message, seconds) in DEMO_SCRIPT {
            write(state, message);
            sleep(seconds);
        }
        store.remove(&session_id);
        println!("Demo finished; simulated session removed.");
        return 0;
    }

    let Some(state) = PetState::parse(state_text) else {
        eprint_line(&format!("claude-airou simulate: unknown state \"{state_text}\""));
        return 2;
    };
    let message = parsed
        .option("message")
        .map(str::to_string)
        .unwrap_or_else(|| default_message(state).to_string());
    write(state, &message);
    0
}

/// Port of `defaultMessage(for:)` — the strings users see in the speech bubble.
fn default_message(state: PetState) -> &'static str {
    match state {
        PetState::Hello => "Hi! Ready when you are",
        PetState::Idle => "",
        PetState::Thinking => "Thinking…",
        PetState::Working => "Reading a file",
        PetState::WaitingApproval => "Approve? Running: git push",
        PetState::NeedsInput => "Waiting for you…",
        PetState::Done => "Done!",
        PetState::Error => "Something failed — recovering…",
    }
}

fn current_dir_string() -> String {
    std::env::current_dir()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string())
}

/// Lists pets: "id\tName the species\tWxH\t<origin>" (+ "skipped: …" to stderr).
pub fn run_list_pets() -> i32 {
    let library = PetLibrary::load();
    for line in list_pets_lines(&library) {
        println!("{line}");
    }
    for problem in &library.load_problems {
        eprint_line(&format!("skipped: {problem}"));
    }
    0
}

fn list_pets_lines(library: &PetLibrary) -> Vec<String> {
    library
        .pets
        .iter()
        .map(|loaded| {
            let definition = &loaded.definition;
            let (width, height) = definition.grid_size();
            let origin = loaded
                .source_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "built-in".to_string());
            format!(
                "{}\t{} the {}\t{width}x{height}\t{origin}",
                definition.id, definition.name, definition.species
            )
        })
        .collect()
}

/// A pet reference is a file path first (tilde-expanded, must not be a directory),
/// then a library id. Mirrors Swift's `loadPet(reference:)`.
fn load_pet(reference: &str) -> Result<PetDefinition, String> {
    let expanded = crate::paths::expand_tilde(reference);
    if expanded.exists() && !expanded.is_dir() {
        return PetDefinition::load(&expanded);
    }
    let library = PetLibrary::load();
    if let Some(loaded) = library.pet_with_id(reference) {
        return Ok(loaded.definition.clone());
    }
    Err(format!("no pet with id \"{reference}\" and no such file"))
}

pub fn run_validate(positional: &[String]) -> i32 {
    emit(&validate_output(positional))
}

fn validate_output(positional: &[String]) -> CommandOutput {
    let Some(path_text) = positional.first() else {
        return err_out("claude-airou validate: missing FILE.json".to_string(), 2);
    };
    let path = crate::paths::expand_tilde(path_text);
    let data = match std::fs::read(&path) {
        Ok(data) => data,
        Err(error) => return err_out(format!("INVALID:\ncould not read file: {error}"), 1),
    };
    let definition = match PetDefinition::decode(&data) {
        Ok(definition) => definition,
        // Swift prints its own DecodingError description; serde's message serves here.
        Err(error) => return err_out(format!("INVALID JSON STRUCTURE: {error}"), 1),
    };
    match definition.validate() {
        Ok(warnings) => {
            let (width, height) = definition.grid_size();
            let mut out = vec![format!(
                "OK: {} ({} the {}), grid {width}x{height}, {} state(s)",
                definition.id,
                definition.name,
                definition.species,
                definition.frames.len()
            )];
            out.extend(warnings.iter().map(|warning| format!("warning: {warning}")));
            CommandOutput { out, err: Vec::new(), code: 0 }
        }
        Err(error) => err_out(format!("INVALID:\n{error}"), 1),
    }
}

/// `render PET_ID|FILE [--out DIR] [--scale N] [--bg #RRGGBB]` — scale 1–64 (default 8).
pub fn run_render(positional: &[String], parsed: &Parsed) -> i32 {
    emit(&render_output(positional, parsed))
}

fn render_output(positional: &[String], parsed: &Parsed) -> CommandOutput {
    let Some(reference) = positional.first() else {
        return err_out("claude-airou render: missing PET_ID or FILE".to_string(), 2);
    };
    let definition = match load_pet(reference) {
        Ok(definition) => definition,
        Err(why) => return err_out(format!("claude-airou render: {why}"), 1),
    };
    if let Err(error) = definition.validate() {
        return err_out(format!("claude-airou render: {error}"), 1);
    }
    let mut scale: u32 = 8;
    if let Some(scale_text) = parsed.option("scale") {
        match scale_text.parse::<i64>() {
            Ok(parsed_scale) if (1..=64).contains(&parsed_scale) => scale = parsed_scale as u32,
            _ => {
                return err_out(
                    format!(
                        "claude-airou render: --scale must be an integer between 1 and 64 (got \"{scale_text}\")"
                    ),
                    2,
                )
            }
        }
    }
    let mut background: Option<PixelColor> = None;
    if let Some(background_text) = parsed.option("bg") {
        match PixelColor::parse(background_text) {
            Some(color) => background = Some(color),
            None => {
                return err_out(
                    format!(
                        "claude-airou render: --bg must be #RRGGBB or #RRGGBBAA (got \"{background_text}\")"
                    ),
                    2,
                )
            }
        }
    }
    let out_text = parsed
        .option("out")
        .map(str::to_string)
        .unwrap_or_else(|| format!("./render-{}", definition.id));
    let output_dir = absolute_path(&crate::paths::expand_tilde(&out_text));
    match crate::render::render_all(&definition, &output_dir, scale, background) {
        Ok(written) => CommandOutput {
            out: vec![
                format!("Rendered {} file(s) to {}", written.len(), output_dir.display()),
                format!("Contact sheet: {}", output_dir.join("sheet.png").display()),
            ],
            err: Vec::new(),
            code: 0,
        },
        Err(why) => err_out(format!("claude-airou render: {why}"), 1),
    }
}

/// Swift's `URL(fileURLWithPath:)` resolves relative paths against the current
/// directory; match that so the printed render paths are absolute.
fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    let base = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let relative = path.strip_prefix(".").unwrap_or(path);
    base.join(relative)
}

/// `preview PET_ID|FILE [--state STATE] [--solid]` — "== state [i] ==" + ascii frames.
pub fn run_preview(positional: &[String], parsed: &Parsed) -> i32 {
    emit(&preview_output(positional, parsed))
}

fn preview_output(positional: &[String], parsed: &Parsed) -> CommandOutput {
    let Some(reference) = positional.first() else {
        return err_out("claude-airou preview: missing PET_ID or FILE".to_string(), 2);
    };
    let definition = match load_pet(reference) {
        Ok(definition) => definition,
        Err(why) => return err_out(format!("claude-airou preview: {why}"), 1),
    };
    let states: Vec<PetState> = match parsed.option("state") {
        Some(state_text) => match PetState::parse(state_text) {
            Some(state) => vec![state],
            None => {
                return err_out(format!("claude-airou preview: unknown state \"{state_text}\""), 2)
            }
        },
        None => PetState::ALL.to_vec(),
    };
    let solid = parsed.has_flag("solid");
    let mut out: Vec<String> = Vec::new();
    for state in states {
        let frames = definition.frames_for(state);
        for (index, frame) in frames.iter().enumerate() {
            out.push(format!("== {} [{index}] ==", state.raw()));
            out.push(crate::render::ascii_art(frame, solid));
        }
    }
    CommandOutput { out, err: Vec::new(), code: 0 }
}

/// Tab-separated session dump incl. effective state, age and usage summary
/// ("ctx 42% 5h 10% 7d 3% status_line").
/// Seconds `snapshot` waits for the overlay to answer (Swift: 5 s deadline).
const SNAPSHOT_TIMEOUT_SECS: f64 = 5.0;
const SNAPSHOT_POLL_INTERVAL_SECS: f64 = 0.1;
/// Name used when `--out` points at a directory (Swift: never replace a directory).
const SNAPSHOT_DEFAULT_FILE_NAME: &str = "claude-airou-snapshot.png";

/// Asks the running overlay to render itself to a PNG (works without screen-recording
/// permission). Port of `runSnapshot`: drop `snapshot.request`, wait up to 5 s for
/// `snapshot.png`, then print its path (or copy it to `--out`).
pub fn run_snapshot(parsed: &Parsed) -> i32 {
    let mut remaining_secs = SNAPSHOT_TIMEOUT_SECS;
    run_snapshot_impl(parsed, &mut || {
        if remaining_secs <= 0.0 {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_secs_f64(SNAPSHOT_POLL_INTERVAL_SECS));
        remaining_secs -= SNAPSHOT_POLL_INTERVAL_SECS;
        true
    })
}

/// `wait_a_little` sleeps one poll interval and returns false once the deadline passed;
/// injected so tests can answer the request themselves without a 5 s wait.
fn run_snapshot_impl(parsed: &Parsed, wait_a_little: &mut dyn FnMut() -> bool) -> i32 {
    let output_path: Option<PathBuf> = parsed.option("out").map(|raw| {
        let expanded = crate::paths::expand_tilde(raw);
        // `--out ~/Desktop` means "put snapshot.png in there", never "replace that directory".
        if expanded.is_dir() {
            expanded.join(SNAPSHOT_DEFAULT_FILE_NAME)
        } else {
            expanded
        }
    });
    let image_path = crate::paths::snapshot_image_file();
    let request_path = crate::paths::snapshot_request_file();
    let prepared = crate::paths::ensure_dir(&crate::paths::root_dir()).and_then(|_| {
        let _ = std::fs::remove_file(&image_path);
        std::fs::write(&request_path, b"")
    });
    if let Err(error) = prepared {
        eprint_line(&format!("claude-airou snapshot: {error}"));
        return 1;
    }
    loop {
        if image_path.is_file() {
            match &output_path {
                Some(output_path) => {
                    let copied = std::fs::read(&image_path)
                        .and_then(|data| crate::state_store::write_atomic(output_path, &data));
                    match copied {
                        Ok(()) => println!("{}", output_path.display()),
                        Err(error) => {
                            eprint_line(&format!(
                                "claude-airou snapshot: could not copy to {}: {error}",
                                output_path.display()
                            ));
                            return 1;
                        }
                    }
                }
                None => println!("{}", image_path.display()),
            }
            return 0;
        }
        if !wait_a_little() {
            break;
        }
    }
    let _ = std::fs::remove_file(&request_path);
    eprint_line("claude-airou snapshot: no answer from the overlay — is `claude-airou run` running?");
    1
}

/// Where a scripted click lands (the content of `click.request`).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // consumed by the overlay
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClickTarget {
    /// The centre of the primary pet.
    Primary,
    /// A content x coordinate in points from the panel's left edge.
    ContentX(f64),
}

/// Parses a `click.request` body the way the Swift overlay does: a number is an x
/// coordinate, the word `primary` is the primary pet, anything else is ignored.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // consumed by the overlay
pub fn parse_click_request(text: &str) -> Option<ClickTarget> {
    let trimmed = text.trim();
    if let Ok(x) = trimmed.parse::<f64>() {
        if x.is_finite() {
            return Some(ClickTarget::ContentX(x));
        }
        return None;
    }
    if trimmed == "primary" {
        return Some(ClickTarget::Primary);
    }
    None
}

/// Scripted click on the running overlay: `claude-airou click primary` or
/// `claude-airou click 42` (x in points). Port of `runClick`: the target is written to
/// `click.request` verbatim; the overlay parses it.
pub fn run_click(positional: &[String]) -> i32 {
    let target = positional.first().map(String::as_str).unwrap_or("primary");
    let written = crate::paths::ensure_dir(&crate::paths::root_dir())
        .and_then(|_| crate::state_store::write_atomic(&crate::paths::click_request_file(), target.as_bytes()));
    match written {
        Ok(()) => 0,
        Err(error) => {
            eprint_line(&format!("claude-airou click: {error}"));
            1
        }
    }
}

pub fn run_status() -> i32 {
    for line in status_lines(&StateStore::default()) {
        println!("{line}");
    }
    0
}

fn status_lines(store: &StateStore) -> Vec<String> {
    let sessions = store.load_all();
    if sessions.is_empty() {
        return vec![format!("No sessions in {}", store.directory.display())];
    }
    let usage_by_session: HashMap<String, SessionUsageSnapshot> = store
        .load_all_usage()
        .into_iter()
        .map(|usage| (usage.session_id.clone(), usage))
        .collect();
    sessions
        .iter()
        .map(|session| {
            let age = session.age_secs() as i64;
            let mut usage_text = String::new();
            if let Some(usage) = usage_by_session.get(&session.session_id) {
                let mut parts: Vec<String> = Vec::new();
                if let Some(ctx) = usage.context_used_percentage {
                    parts.push(format!("ctx {}%", ctx.round() as i64));
                }
                if let Some(five) = usage.five_hour_used_percentage {
                    parts.push(format!("5h {}%", five.round() as i64));
                }
                if let Some(seven) = usage.seven_day_used_percentage {
                    parts.push(format!("7d {}%", seven.round() as i64));
                }
                if let Some(effort) = usage.effort_level {
                    // Also what drives the aura behind the pet, so `status` is how you check it.
                    parts.push(format!("effort {}", effort.raw()));
                }
                parts.push(usage.source.raw().to_string());
                usage_text = format!("\t[{}]", parts.join(" "));
            }
            format!(
                "{}\t{}\t{} → {}\t{age}s ago\t{}{usage_text}",
                session.session_id,
                session.project_name(),
                session.state.raw(),
                session.effective_state().raw(),
                session.message
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::UsageSource;
    use std::cell::RefCell;
    use std::sync::{Mutex, MutexGuard};

    /// Tests that set (or depend on) `CLAUDE_AIROU_HOME` must not run in parallel.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        previous: Option<String>,
        _lock: MutexGuard<'static, ()>,
    }

    impl EnvGuard {
        fn set_home(value: &Path) -> EnvGuard {
            let lock = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let previous = std::env::var("CLAUDE_AIROU_HOME").ok();
            std::env::set_var("CLAUDE_AIROU_HOME", value);
            EnvGuard { previous, _lock: lock }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var("CLAUDE_AIROU_HOME", value),
                None => std::env::remove_var("CLAUDE_AIROU_HOME"),
            }
        }
    }

    fn parsed(options: &[(&str, &str)], flags: &[&str]) -> Parsed {
        let mut result = Parsed::default();
        for (key, value) in options {
            result.options.insert(key.to_string(), value.to_string());
        }
        for flag in flags {
            result.flags.insert(flag.to_string());
        }
        result
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|item| item.to_string()).collect()
    }

    fn fixture_pet_json(id: &str) -> String {
        format!(
            r##"{{
              "id": "{id}", "name": "Pixel", "species": "blob",
              "palette": {{ "k": "#112233" }},
              "frames": {{ "idle": [["kkkk", "k..k", "k..k", "kkkk"]] }}
            }}"##
        )
    }

    fn write_fixture(dir: &Path, name: &str, id: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, fixture_pet_json(id)).unwrap();
        path
    }

    // MARK: simulate

    #[test]
    fn default_messages_match_swift() {
        assert_eq!(default_message(PetState::Hello), "Hi! Ready when you are");
        assert_eq!(default_message(PetState::Idle), "");
        assert_eq!(default_message(PetState::Thinking), "Thinking…");
        assert_eq!(default_message(PetState::Working), "Reading a file");
        assert_eq!(default_message(PetState::WaitingApproval), "Approve? Running: git push");
        assert_eq!(default_message(PetState::NeedsInput), "Waiting for you…");
        assert_eq!(default_message(PetState::Done), "Done!");
        assert_eq!(default_message(PetState::Error), "Something failed — recovering…");
    }

    #[test]
    fn demo_script_is_the_expected_sequence() {
        let expected: [(PetState, &str, f64); 10] = [
            (PetState::Hello, "Hi! Ready when you are", 3.0),
            (PetState::Thinking, "Thinking…", 3.0),
            (PetState::Working, "Reading main.rs", 3.0),
            (PetState::Working, "Running: cargo build", 3.0),
            (PetState::WaitingApproval, "Approve? Running: git push", 5.0),
            (PetState::Working, "Editing README.md", 3.0),
            (PetState::Error, "Bash failed — recovering…", 3.0),
            (PetState::Thinking, "Thinking…", 2.0),
            (PetState::Done, "Done!", 4.0),
            (PetState::NeedsInput, "Waiting for you…", 4.0),
        ];
        assert_eq!(DEMO_SCRIPT, expected);
    }

    #[test]
    fn simulate_missing_state_exits_2() {
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set_home(dir.path());
        assert_eq!(run_simulate(&[], &parsed(&[], &[])), 2);
        assert!(!dir.path().join("state").exists());
    }

    #[test]
    fn simulate_unknown_state_exits_2_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set_home(dir.path());
        assert_eq!(run_simulate(&args(&["sparkle"]), &parsed(&[], &[])), 2);
        assert!(!dir.path().join("state").join("simulated.json").exists());
    }

    #[test]
    fn simulate_writes_snapshot_with_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set_home(dir.path());
        assert_eq!(run_simulate(&args(&["working"]), &parsed(&[], &[])), 0);

        let store = StateStore::default();
        let snapshot = store.read("simulated").expect("snapshot written");
        assert_eq!(snapshot.session_id, "simulated");
        assert_eq!(snapshot.state, PetState::Working);
        assert_eq!(snapshot.message, "Reading a file");
        assert_eq!(snapshot.last_event_name, "simulate");
        assert_eq!(snapshot.tool_name, None);
        assert_eq!(snapshot.pending_tool_use_id, None);
        assert_eq!(snapshot.cwd, current_dir_string());
        assert!((now_epoch_secs() - snapshot.updated_at_epoch_seconds).abs() < 30.0);
    }

    #[test]
    fn simulate_honors_session_message_cwd_and_aliases() {
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set_home(dir.path());
        let options = parsed(
            &[("session", "abc"), ("message", "custom text"), ("cwd", "/tmp/some/project")],
            &[],
        );
        // "ok" is a Swift-side alias for done; PetState::parse handles it.
        assert_eq!(run_simulate(&args(&["ok"]), &options), 0);

        let snapshot = StateStore::default().read("abc").expect("snapshot written");
        assert_eq!(snapshot.state, PetState::Done);
        assert_eq!(snapshot.message, "custom text");
        assert_eq!(snapshot.cwd, "/tmp/some/project");
        assert_eq!(snapshot.project_name(), "project");
    }

    #[test]
    fn simulate_clear_removes_session_and_usage() {
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set_home(dir.path());
        assert_eq!(run_simulate(&args(&["done"]), &parsed(&[("session", "s1")], &[])), 0);
        let store = StateStore::default();
        std::fs::write(store.usage_file_url("s1"), b"{}").unwrap();
        assert!(store.file_url("s1").exists());

        assert_eq!(run_simulate(&args(&["clear"]), &parsed(&[("session", "s1")], &[])), 0);
        assert!(!store.file_url("s1").exists());
        assert!(!store.usage_file_url("s1").exists());

        // Clearing a session that never existed still succeeds (remove is best-effort).
        assert_eq!(run_simulate(&args(&["clear"]), &parsed(&[], &[])), 0);
    }

    #[test]
    fn simulate_demo_runs_script_then_removes_session() {
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set_home(dir.path());
        let observed: RefCell<Vec<(f64, PetState, String)>> = RefCell::new(Vec::new());
        let store = StateStore::default();
        let sleeper = |seconds: f64| {
            // Called right after each write: capture what is on disk at that moment.
            let snapshot = store.read("demo-session").expect("demo snapshot on disk");
            observed.borrow_mut().push((seconds, snapshot.state, snapshot.message));
        };

        let code = run_simulate_impl(
            &args(&["demo"]),
            &parsed(&[("session", "demo-session")], &[]),
            &sleeper,
        );
        assert_eq!(code, 0);

        let observed = observed.into_inner();
        assert_eq!(observed.len(), 10);
        for (index, (state, message, seconds)) in DEMO_SCRIPT.iter().enumerate() {
            assert_eq!(observed[index].0, *seconds, "sleep {index}");
            assert_eq!(observed[index].1, *state, "state {index}");
            assert_eq!(observed[index].2, *message, "message {index}");
        }
        // The session file is removed once the demo finishes.
        assert!(store.read("demo-session").is_none());
        assert!(!store.file_url("demo-session").exists());
    }

    // MARK: pets / load_pet

    #[test]
    fn list_pets_lines_format_built_in_and_user() {
        let pets_dir = tempfile::tempdir().unwrap();
        write_fixture(pets_dir.path(), "zeta.json", "zeta-pet");
        std::fs::write(pets_dir.path().join("broken.json"), b"{ nope").unwrap();
        let library = PetLibrary::load_from(pets_dir.path());

        let lines = list_pets_lines(&library);
        assert_eq!(lines.len(), 9); // 8 built-ins + 1 user pet
        assert!(lines[0].starts_with("airou-felyne\t"));
        assert!(lines[0].ends_with("\tbuilt-in"));
        // Exactly four tab-separated columns per line.
        for line in &lines {
            assert_eq!(line.matches('\t').count(), 3, "line: {line}");
        }
        let zeta = lines.last().unwrap();
        assert_eq!(
            zeta,
            &format!("zeta-pet\tPixel the blob\t4x4\t{}", pets_dir.path().join("zeta.json").display())
        );
        assert_eq!(library.load_problems.len(), 1);
        assert!(library.load_problems[0].starts_with("broken.json: "));
    }

    #[test]
    fn load_pet_prefers_file_then_library_then_errors() {
        let home = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set_home(home.path());
        let files = tempfile::tempdir().unwrap();
        let path = write_fixture(files.path(), "custom.json", "file-pet");

        // 1. Existing file wins.
        let definition = load_pet(path.to_str().unwrap()).unwrap();
        assert_eq!(definition.id, "file-pet");

        // 2. Otherwise a library id (built-in here; CLAUDE_AIROU_HOME is empty).
        let definition = load_pet("mochi-cat").unwrap();
        assert_eq!(definition.id, "mochi-cat");

        // 3. A directory path never counts as a file and is no id either.
        let dir_text = files.path().to_str().unwrap().to_string();
        let error = load_pet(&dir_text).unwrap_err();
        assert_eq!(error, format!("no pet with id \"{dir_text}\" and no such file"));

        // 4. Unknown id, no such file.
        let error = load_pet("nope-xyz").unwrap_err();
        assert_eq!(error, "no pet with id \"nope-xyz\" and no such file");
    }

    // MARK: validate

    #[test]
    fn validate_missing_arg_exits_2() {
        let output = validate_output(&[]);
        assert_eq!(output.code, 2);
        assert_eq!(output.err, vec!["claude-airou validate: missing FILE.json"]);
        assert!(output.out.is_empty());
    }

    #[test]
    fn validate_ok_line_and_warnings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pet.json");
        std::fs::write(
            &path,
            r##"{
              "id": "warny", "name": "Warny", "species": "owl",
              "palette": { "k": "#112233", "u": "#FFFFFF" },
              "frames": {
                "idle": [["kkkk", "k..k", "k..k", "kkkk"]],
                "working": [["....", "kkkk", "kkkk", "...."]]
              }
            }"##,
        )
        .unwrap();

        let output = validate_output(&args(&[path.to_str().unwrap()]));
        assert_eq!(output.code, 0);
        assert!(output.err.is_empty());
        assert_eq!(output.out[0], "OK: warny (Warny the owl), grid 4x4, 2 state(s)");
        let warnings: Vec<&String> = output.out.iter().skip(1).collect();
        assert!(!warnings.is_empty());
        assert!(warnings.iter().all(|line| line.starts_with("warning: ")));
        assert!(output.out.contains(&"warning: palette key \"u\" is never used".to_string()));
        assert!(output
            .out
            .contains(&"warning: no frames for hello; falling back to done".to_string()));
    }

    #[test]
    fn validate_json_structure_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, br#"{"id": 3}"#).unwrap();
        let output = validate_output(&args(&[path.to_str().unwrap()]));
        assert_eq!(output.code, 1);
        assert_eq!(output.err.len(), 1);
        assert!(output.err[0].starts_with("INVALID JSON STRUCTURE: "), "{}", output.err[0]);
        assert!(output.out.is_empty());
    }

    #[test]
    fn validate_invalid_pet_reports_problems() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invalid.json");
        std::fs::write(
            &path,
            r#"{
              "id": "bad pet!", "name": "Bad", "species": "blob",
              "palette": { "k": "green" },
              "frames": { "idle": [["kkkk", "k..k", "k..k", "kkkk"]] }
            }"#,
        )
        .unwrap();
        let output = validate_output(&args(&[path.to_str().unwrap()]));
        assert_eq!(output.code, 1);
        assert_eq!(output.err.len(), 1);
        assert!(output.err[0].starts_with("INVALID:\n"));
        assert!(output.err[0]
            .contains("`id` may only contain letters, digits, '-' and '_' (got \"bad pet!\")"));
        assert!(output.err[0]
            .contains("palette[\"k\"] = \"green\" is not a #RRGGBB / #RRGGBBAA color"));
    }

    #[test]
    fn validate_unreadable_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.json");
        let output = validate_output(&args(&[path.to_str().unwrap()]));
        assert_eq!(output.code, 1);
        assert!(output.err[0].starts_with("INVALID:\ncould not read file: "), "{}", output.err[0]);
    }

    // MARK: render

    #[test]
    fn render_missing_reference_exits_2() {
        let output = render_output(&[], &parsed(&[], &[]));
        assert_eq!(output.code, 2);
        assert_eq!(output.err, vec!["claude-airou render: missing PET_ID or FILE"]);
    }

    #[test]
    fn render_unknown_pet_exits_1() {
        let home = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set_home(home.path());
        let output = render_output(&args(&["nope-xyz"]), &parsed(&[], &[]));
        assert_eq!(output.code, 1);
        assert_eq!(
            output.err,
            vec!["claude-airou render: no pet with id \"nope-xyz\" and no such file"]
        );
    }

    #[test]
    fn render_scale_out_of_range_or_junk_exits_2() {
        let files = tempfile::tempdir().unwrap();
        let path = write_fixture(files.path(), "pet.json", "tiny-pet");
        let reference = path.to_str().unwrap();

        for bad in ["65", "0", "abc", "8.5"] {
            let output = render_output(&args(&[reference]), &parsed(&[("scale", bad)], &[]));
            assert_eq!(output.code, 2, "scale {bad}");
            assert_eq!(
                output.err,
                vec![format!(
                    "claude-airou render: --scale must be an integer between 1 and 64 (got \"{bad}\")"
                )]
            );
        }
    }

    #[test]
    fn render_bad_background_exits_2() {
        let files = tempfile::tempdir().unwrap();
        let path = write_fixture(files.path(), "pet.json", "tiny-pet");
        let output = render_output(
            &args(&[path.to_str().unwrap()]),
            &parsed(&[("bg", "green")], &[]),
        );
        assert_eq!(output.code, 2);
        assert_eq!(
            output.err,
            vec!["claude-airou render: --bg must be #RRGGBB or #RRGGBBAA (got \"green\")"]
        );
    }

    #[test]
    fn render_writes_pngs_and_prints_summary() {
        let files = tempfile::tempdir().unwrap();
        let path = write_fixture(files.path(), "pet.json", "tiny-pet");
        let out_dir = files.path().join("out");
        let output = render_output(
            &args(&[path.to_str().unwrap()]),
            &parsed(&[("out", out_dir.to_str().unwrap()), ("scale", "2"), ("bg", "#10203040")], &[]),
        );
        assert_eq!(output.code, 0, "err: {:?}", output.err);
        // One idle frame backs all 8 states via fallback, plus sheet.png = 9 files.
        assert_eq!(
            output.out,
            vec![
                format!("Rendered 9 file(s) to {}", out_dir.display()),
                format!("Contact sheet: {}", out_dir.join("sheet.png").display()),
            ]
        );
        assert!(out_dir.join("sheet.png").exists());
        assert!(out_dir.join("idle_0.png").exists());
        assert!(out_dir.join("waiting_approval_0.png").exists());
    }

    #[test]
    fn render_invalid_pet_file_fails_validation_first() {
        let files = tempfile::tempdir().unwrap();
        let path = files.path().join("invalid.json");
        std::fs::write(
            &path,
            r##"{
              "id": "small", "name": "Small", "species": "dot",
              "palette": { "k": "#112233" },
              "frames": { "idle": [["kk", "kk"]] }
            }"##,
        )
        .unwrap();
        // Invalid grid beats the bad --scale: Swift validates before parsing options.
        let output = render_output(&args(&[path.to_str().unwrap()]), &parsed(&[("scale", "999")], &[]));
        assert_eq!(output.code, 1);
        assert_eq!(
            output.err,
            vec!["claude-airou render: grid must be at least 4x4 (got 2x2)"]
        );
    }

    #[test]
    fn absolute_path_resolves_like_swift_file_url() {
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(absolute_path(Path::new("/abs/dir")), PathBuf::from("/abs/dir"));
        assert_eq!(absolute_path(Path::new("./render-x")), cwd.join("render-x"));
        assert_eq!(absolute_path(Path::new("render-x")), cwd.join("render-x"));
    }

    // MARK: preview

    #[test]
    fn preview_missing_reference_exits_2() {
        let output = preview_output(&[], &parsed(&[], &[]));
        assert_eq!(output.code, 2);
        assert_eq!(output.err, vec!["claude-airou preview: missing PET_ID or FILE"]);
    }

    #[test]
    fn preview_unknown_state_exits_2() {
        let files = tempfile::tempdir().unwrap();
        let path = write_fixture(files.path(), "pet.json", "tiny-pet");
        let output = preview_output(
            &args(&[path.to_str().unwrap()]),
            &parsed(&[("state", "sparkle")], &[]),
        );
        assert_eq!(output.code, 2);
        assert_eq!(output.err, vec!["claude-airou preview: unknown state \"sparkle\""]);
    }

    #[test]
    fn preview_single_state_header_and_ascii() {
        let files = tempfile::tempdir().unwrap();
        let path = write_fixture(files.path(), "pet.json", "tiny-pet");
        let output = preview_output(
            &args(&[path.to_str().unwrap()]),
            &parsed(&[("state", "working")], &[]),
        );
        assert_eq!(output.code, 0);
        // working falls back to idle's single frame.
        assert_eq!(
            output.out,
            vec!["== working [0] ==".to_string(), "kkkk\nk  k\nk  k\nkkkk".to_string()]
        );

        let solid = preview_output(
            &args(&[path.to_str().unwrap()]),
            &parsed(&[("state", "working")], &["solid"]),
        );
        assert_eq!(solid.out[1], "####\n#  #\n#  #\n####");
    }

    #[test]
    fn preview_all_states_in_order() {
        let files = tempfile::tempdir().unwrap();
        let path = write_fixture(files.path(), "pet.json", "tiny-pet");
        let output = preview_output(&args(&[path.to_str().unwrap()]), &parsed(&[], &[]));
        assert_eq!(output.code, 0);
        // 8 states x (header + one fallback frame each) = 16 lines.
        assert_eq!(output.out.len(), 16);
        let headers: Vec<&String> = output.out.iter().step_by(2).collect();
        let expected: Vec<String> = PetState::ALL
            .iter()
            .map(|state| format!("== {} [0] ==", state.raw()))
            .collect();
        assert_eq!(headers, expected.iter().collect::<Vec<_>>());
    }

    // MARK: status

    fn session(id: &str, cwd: &str, state: PetState, message: &str, age: f64) -> SessionSnapshot {
        SessionSnapshot {
            session_id: id.to_string(),
            cwd: cwd.to_string(),
            state,
            message: message.to_string(),
            last_event_name: "test".to_string(),
            tool_name: None,
            updated_at_epoch_seconds: now_epoch_secs() - age,
            pending_tool_use_id: None,
            active_agent_ids: Vec::new(),
        }
    }

    fn usage(id: &str, source: UsageSource) -> SessionUsageSnapshot {
        SessionUsageSnapshot {
            session_id: id.to_string(),
            source,
            effort_level: None,
            updated_at_epoch_seconds: now_epoch_secs(),
            context_used_percentage: None,
            context_window_size: None,
            context_tokens: None,
            total_input_tokens: None,
            total_output_tokens: None,
            model_display_name: None,
            five_hour_used_percentage: None,
            five_hour_resets_at_epoch_seconds: None,
            seven_day_used_percentage: None,
            seven_day_resets_at_epoch_seconds: None,
            total_cost_usd: None,
        }
    }

    #[test]
    fn status_empty_dir_message() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path().to_path_buf());
        assert_eq!(
            status_lines(&store),
            vec![format!("No sessions in {}", dir.path().display())]
        );
    }

    #[test]
    fn status_lines_with_and_without_usage() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path().to_path_buf());

        // Newest session: working, full usage suffix. 42.4 -> 42, 9.5 -> 10, 2.6 -> 3.
        store
            .write(&session("s-new", "/Users/me/proj", PetState::Working, "Reading a file", 2.2))
            .unwrap();
        let mut full = usage("s-new", UsageSource::StatusLine);
        full.context_used_percentage = Some(42.4);
        full.five_hour_used_percentage = Some(9.5);
        full.seven_day_used_percentage = Some(2.6);
        store.write_usage(&full).unwrap();

        // Older session: hello already decayed to idle, no usage file.
        store
            .write(&session("s-old", "/work/other", PetState::Hello, "Hi!", 10.2))
            .unwrap();

        // Third: partial usage (only 5h) from a transcript estimate.
        store
            .write(&session("s-mid", "/work/mid", PetState::Done, "Done!", 5.0))
            .unwrap();
        let mut partial = usage("s-mid", UsageSource::Transcript);
        partial.five_hour_used_percentage = Some(12.0);
        store.write_usage(&partial).unwrap();

        let lines = status_lines(&store);
        assert_eq!(lines.len(), 3);
        // Newest first (store sorts by updatedAt descending).
        assert_eq!(
            lines[0],
            "s-new\tproj\tworking → working\t2s ago\tReading a file\t[ctx 42% 5h 10% 7d 3% status_line]"
        );
        assert_eq!(lines[1], "s-mid\tmid\tdone → done\t5s ago\tDone!\t[5h 12% transcript]");
        assert_eq!(lines[2], "s-old\tother\thello → idle\t10s ago\tHi!");
    }

    #[test]
    fn click_request_parsing_matches_swift() {
        assert_eq!(parse_click_request("primary"), Some(ClickTarget::Primary));
        assert_eq!(parse_click_request("  primary\n"), Some(ClickTarget::Primary));
        assert_eq!(parse_click_request("42"), Some(ClickTarget::ContentX(42.0)));
        assert_eq!(parse_click_request("117.5\n"), Some(ClickTarget::ContentX(117.5)));
        assert_eq!(parse_click_request("-3"), Some(ClickTarget::ContentX(-3.0)));
        assert_eq!(parse_click_request("nan"), None, "not finite");
        assert_eq!(parse_click_request("inf"), None);
        assert_eq!(parse_click_request("PRIMARY"), None);
        assert_eq!(parse_click_request("left"), None);
        assert_eq!(parse_click_request(""), None);
    }

    #[test]
    fn click_writes_request_file_defaulting_to_primary() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = EnvGuard::set_home(dir.path());
        assert_eq!(run_click(&[]), 0);
        let request = dir.path().join("click.request");
        assert_eq!(std::fs::read_to_string(&request).unwrap(), "primary");
        assert_eq!(run_click(&args(&["135"])), 0);
        assert_eq!(std::fs::read_to_string(&request).unwrap(), "135");
        assert_eq!(parse_click_request(&std::fs::read_to_string(&request).unwrap()), Some(ClickTarget::ContentX(135.0)));
    }

    #[test]
    fn snapshot_times_out_without_an_overlay_and_removes_the_request() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = EnvGuard::set_home(dir.path());
        let mut polls = 0;
        let code = run_snapshot_impl(&parsed(&[], &[]), &mut || {
            polls += 1;
            polls < 3
        });
        assert_eq!(code, 1);
        assert_eq!(polls, 3);
        assert!(!dir.path().join("snapshot.request").exists(), "request withdrawn on timeout");
    }

    #[test]
    fn snapshot_prints_image_path_when_the_overlay_answers() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = EnvGuard::set_home(dir.path());
        // Stale image from an earlier run must be removed before asking.
        std::fs::write(dir.path().join("snapshot.png"), b"stale").unwrap();
        let image_path = dir.path().join("snapshot.png");
        let request_path = dir.path().join("snapshot.request");
        let mut answered = false;
        let code = run_snapshot_impl(&parsed(&[], &[]), &mut || {
            assert!(request_path.exists(), "request file is dropped first");
            assert!(!answered || image_path.exists());
            // Play the overlay: consume the request, write the PNG.
            std::fs::remove_file(&request_path).unwrap();
            std::fs::write(&image_path, b"png-bytes").unwrap();
            answered = true;
            true
        });
        assert_eq!(code, 0);
        assert!(answered);
        assert_eq!(std::fs::read(&image_path).unwrap(), b"png-bytes");
    }

    #[test]
    fn snapshot_out_copies_into_directory_or_file() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = EnvGuard::set_home(dir.path());
        let image_path = dir.path().join("snapshot.png");
        let out_dir = dir.path().join("out");
        std::fs::create_dir_all(&out_dir).unwrap();
        let mut answer = || {
            std::fs::write(&image_path, b"png-bytes").unwrap();
            true
        };
        let code = run_snapshot_impl(&parsed(&[("out", out_dir.to_str().unwrap())], &[]), &mut answer);
        assert_eq!(code, 0);
        assert_eq!(std::fs::read(out_dir.join("claude-airou-snapshot.png")).unwrap(), b"png-bytes");
        assert!(out_dir.is_dir(), "a directory is never replaced");

        let out_file = dir.path().join("shot.png");
        let code = run_snapshot_impl(&parsed(&[("out", out_file.to_str().unwrap())], &[]), &mut answer);
        assert_eq!(code, 0);
        assert_eq!(std::fs::read(&out_file).unwrap(), b"png-bytes");
    }
}
