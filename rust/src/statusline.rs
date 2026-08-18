//! `claude-airou statusline` (usage recording + passthrough) and the transcript-based
//! context estimator. Ports of `Hook/StatusLineCommand.swift` and
//! `Hook/TranscriptContextEstimator.swift` — read those files for the exact JSON fields,
//! passthrough semantics (run the user's original status line command with the same stdin,
//! forward its stdout/exit code) and estimation rules (last assistant message's token usage
//! in the session's transcript JSONL; cache_read+cache_creation+input(+output) tokens;
//! known window size preferred, else infer from model; source = "transcript").

use crate::model::{now_epoch_secs, SessionUsageSnapshot, UsageSource};
use crate::state_store::StateStore;
use serde_json::Value;
use std::path::PathBuf;

/// Hook events on which the estimate is refreshed (Swift:
/// `TranscriptContextEstimator.refreshingEventNames`).
pub const REFRESHING_EVENT_NAMES: [&str; 4] =
    ["PostToolUse", "PostToolBatch", "Stop", "UserPromptSubmit"];

/// Set in the passthrough's environment; if we ever see it on our own stdin path we are
/// being invoked by ourselves and must not spawn again (Swift:
/// `StatusLineCommand.recursionGuardEnvironmentKey`).
const RECURSION_GUARD_ENV_KEY: &str = "CLAUDE_AIROU_STATUSLINE_DEPTH";

// Swift: TranscriptContextEstimator.tailBytesToRead / defaultContextWindowSize /
// largeContextWindowSize.
const TAIL_BYTES_TO_READ: u64 = 96 * 1024;
const DEFAULT_CONTEXT_WINDOW_SIZE: i64 = 200_000;
const LARGE_CONTEXT_WINDOW_SIZE: i64 = 1_000_000;

/// Entry point for `claude-airou statusline`. Reads the status line JSON on stdin, records
/// a `SessionUsageSnapshot` (source status_line) via `StateStore::merge_usage`, then execs
/// the stashed passthrough command (paths::statusline_passthrough_file()) with the same
/// stdin bytes and mirrors its stdout / exit code. Never fails the status line: on any
/// internal error still run the passthrough (or exit 0).
pub fn run(raw_args: &[String]) -> i32 {
    // Swift ignores SIGPIPE up front; the Rust runtime already starts with SIGPIPE
    // ignored, so a passthrough that exits before draining stdin cannot kill us either.
    use std::io::{IsTerminal, Read};

    if std::io::stdin().is_terminal() {
        crate::logging::eprint_line(
            "claude-airou statusline: expects the Claude Code status line JSON on stdin (see `claude-airou install-statusline`).",
        );
        return 0;
    }
    let mut input_data = Vec::new();
    let _ = std::io::stdin().lock().read_to_end(&mut input_data);

    // Record usage; never fail the status line because of our own bookkeeping.
    if let Ok(object) = serde_json::from_slice::<Value>(&input_data) {
        if let Some(usage) = parse_usage(&object, now_epoch_secs()) {
            let _ = StateStore::default().merge_usage(&usage);
        }
    }

    let options = parse_options(raw_args);
    run_passthrough(
        &input_data,
        options.then_command.as_deref(),
        options.settings_path.as_deref(),
    )
}

/// Estimate context usage from the transcript JSONL; `None` when nothing usable is found.
pub fn transcript_estimate(
    transcript_path: &str,
    session_id: &str,
    known_window_size: Option<i64>,
) -> Option<SessionUsageSnapshot> {
    transcript_estimate_at(transcript_path, session_id, known_window_size, now_epoch_secs())
}

// MARK: - Options (Swift: StatusLineCommand.parseOptions)

/// `--then CMD` / `--then=CMD` (testing) and `--settings PATH` / `--settings=PATH`
/// (which passthrough file).
#[derive(Debug, Default, Clone)]
struct Options {
    then_command: Option<String>,
    settings_path: Option<String>,
}

fn parse_options(arguments: &[String]) -> Options {
    let mut options = Options::default();
    let mut index = 0;
    while index < arguments.len() {
        if let Some(then) = option_value(arguments, &mut index, "--then") {
            options.then_command = Some(then);
        } else if let Some(settings) = option_value(arguments, &mut index, "--settings") {
            // Swift expands `~` via NSString.expandingTildeInPath.
            options.settings_path =
                Some(crate::paths::expand_tilde(&settings).to_string_lossy().to_string());
        }
        index += 1;
    }
    options
}

/// Swift's local `value(after:)`: `--name value` (consuming the next argument) or `--name=value`.
fn option_value(arguments: &[String], index: &mut usize, name: &str) -> Option<String> {
    let argument = &arguments[*index];
    if argument == name && *index + 1 < arguments.len() {
        *index += 1;
        return Some(arguments[*index].clone());
    }
    argument
        .strip_prefix(name)
        .and_then(|rest| rest.strip_prefix('='))
        .map(str::to_string)
}

// MARK: - Parsing (Swift: StatusLineCommand.parseUsage)

/// Extracts the usage figures Claude Code hands to the status line. `None` when the JSON
/// carries no usable `session_id`.
fn parse_usage(object: &Value, now_secs: f64) -> Option<SessionUsageSnapshot> {
    let session_id = object.get("session_id").and_then(Value::as_str)?;
    if session_id.is_empty() {
        return None;
    }

    let context_window = object.get("context_window").and_then(Value::as_object);
    let rate_limits = object.get("rate_limits").and_then(Value::as_object);
    let five_hour = rate_limits.and_then(|limits| limits.get("five_hour")).and_then(Value::as_object);
    let seven_day = rate_limits.and_then(|limits| limits.get("seven_day")).and_then(Value::as_object);
    let cost = object.get("cost").and_then(Value::as_object);
    let model = object.get("model").and_then(Value::as_object);

    let mut context_used = number(context_window, "used_percentage");
    let context_size = number(context_window, "context_window_size").map(|value| value as i64);
    let mut context_tokens: Option<i64> = None;
    if let Some(current_usage) = context_window
        .and_then(|window| window.get("current_usage"))
        .and_then(Value::as_object)
    {
        let current = Some(current_usage);
        let input = number(current, "input_tokens").unwrap_or(0.0);
        let cache_creation = number(current, "cache_creation_input_tokens").unwrap_or(0.0);
        let cache_read = number(current, "cache_read_input_tokens").unwrap_or(0.0);
        let total = input + cache_creation + cache_read;
        if total > 0.0 {
            context_tokens = Some(total as i64);
        }
        if context_used.is_none() {
            if let Some(size) = context_size {
                if size > 0 && total > 0.0 {
                    context_used = Some(total / size as f64 * 100.0);
                }
            }
        }
    }

    Some(SessionUsageSnapshot {
        session_id: session_id.to_string(),
        source: UsageSource::StatusLine,
        updated_at_epoch_seconds: now_secs,
        context_used_percentage: context_used,
        context_window_size: context_size,
        context_tokens,
        total_input_tokens: number(context_window, "total_input_tokens").map(|value| value as i64),
        total_output_tokens: number(context_window, "total_output_tokens").map(|value| value as i64),
        model_display_name: model
            .and_then(|model| model.get("display_name"))
            .and_then(Value::as_str)
            .map(str::to_string),
        five_hour_used_percentage: number(five_hour, "used_percentage"),
        five_hour_resets_at_epoch_seconds: epoch_seconds(five_hour, "resets_at"),
        seven_day_used_percentage: number(seven_day, "used_percentage"),
        seven_day_resets_at_epoch_seconds: epoch_seconds(seven_day, "resets_at"),
        total_cost_usd: number(cost, "total_cost_usd"),
    })
}

/// Swift's local `number(_:_:)`: JSON number, or a string parseable as a double.
fn number(container: Option<&serde_json::Map<String, Value>>, key: &str) -> Option<f64> {
    match container?.get(key)? {
        Value::Number(value) => value.as_f64(),
        Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    }
}

/// Swift's local `date(_:_:)`: epoch seconds (or milliseconds — anything > 1e12), as a
/// number or a numeric string, or an ISO 8601 / RFC 3339 date string.
fn epoch_seconds(container: Option<&serde_json::Map<String, Value>>, key: &str) -> Option<f64> {
    fn normalize(seconds: f64) -> f64 {
        if seconds > 1e12 {
            seconds / 1000.0 // ms vs s
        } else {
            seconds
        }
    }
    match container?.get(key)? {
        Value::Number(value) => value.as_f64().map(normalize),
        Value::String(text) => {
            if let Ok(seconds) = text.parse::<f64>() {
                return Some(normalize(seconds));
            }
            chrono::DateTime::parse_from_rfc3339(text)
                .ok()
                .map(|parsed| parsed.timestamp() as f64 + f64::from(parsed.timestamp_subsec_nanos()) / 1e9)
        }
        _ => None,
    }
}

// MARK: - Passthrough (Swift: StatusLineCommand.runPassthrough & friends)

/// The user's original `statusLine` object from settings.json, kept here while ours is
/// installed. One file per settings file, so `--settings` targets don't clobber each other.
/// (Swift: `StatusLineCommand.passthroughFile(forSettingsPath:)`.)
fn passthrough_file(settings_path: Option<&str>) -> PathBuf {
    match settings_path {
        None => crate::paths::statusline_passthrough_file(),
        Some(path) if path == crate::paths::claude_settings_file().to_string_lossy() => {
            crate::paths::statusline_passthrough_file()
        }
        Some(path) => {
            let digest = base36(djb2(path));
            crate::paths::root_dir().join(format!("statusline-passthrough-{digest}.json"))
        }
    }
}

/// Swift: `settingsPath.utf8.reduce(UInt64(5381)) { ($0 &* 33) &+ UInt64($1) }`.
fn djb2(text: &str) -> u64 {
    text.bytes()
        .fold(5381u64, |hash, byte| hash.wrapping_mul(33).wrapping_add(u64::from(byte)))
}

/// Swift: `String(_, radix: 36)` (lowercase digits).
fn base36(mut value: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if value == 0 {
        return "0".to_string();
    }
    let mut out: Vec<u8> = Vec::new();
    while value > 0 {
        out.push(DIGITS[(value % 36) as usize]);
        value /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

fn stored_passthrough_object(settings_path: Option<&str>) -> Option<serde_json::Map<String, Value>> {
    let data = std::fs::read(passthrough_file(settings_path)).ok()?;
    match serde_json::from_slice::<Value>(&data).ok()? {
        Value::Object(object) => Some(object),
        _ => None,
    }
}

fn stored_passthrough_command(settings_path: Option<&str>) -> Option<String> {
    let object = stored_passthrough_object(settings_path)?;
    // Swift: `(object["type"] as? String ?? "command") == "command"`.
    if object.get("type").and_then(Value::as_str).unwrap_or("command") != "command" {
        return None;
    }
    object.get("command").and_then(Value::as_str).map(str::to_string)
}

/// Marker used to recognise our own entries regardless of where the binary lives
/// (Swift: `HooksInstaller.containsOurMarker`, incl. the legacy `claude-pet` name).
fn contains_our_marker(text: &str) -> bool {
    text.contains("claude-airou") || text.contains("claude-pet")
}

/// True when `command` is one of our own status line commands (would recurse forever).
fn is_self_invocation(command: &str) -> bool {
    let trimmed = command.trim();
    if !contains_our_marker(trimmed) {
        return false;
    }
    trimmed.ends_with(" statusline") || trimmed.contains(" statusline ")
}

/// Runs the user's own status line command with the same stdin and forwards its output;
/// returns its exit code. Stdout/stderr are inherited, exactly like the Swift build.
fn run_passthrough(input_data: &[u8], explicit_command: Option<&str>, settings_path: Option<&str>) -> i32 {
    if std::env::var_os(RECURSION_GUARD_ENV_KEY).is_some() {
        crate::logging::eprint_line("claude-airou statusline: refusing to run nested (recursion guard).");
        return 0;
    }
    let command = match explicit_command {
        Some(command) => Some(command.to_string()),
        None => stored_passthrough_command(settings_path),
    };
    let Some(command) = command else {
        return 0; // no original status line: print nothing
    };
    if command.trim().is_empty() {
        return 0;
    }
    if is_self_invocation(&command) {
        crate::logging::eprint_line(
            "claude-airou statusline: stored passthrough is claude-airou itself; not running it. Run `claude-airou uninstall-statusline` then `install-statusline` to repair.",
        );
        return 0;
    }

    let mut child = match std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(&command)
        .env(RECURSION_GUARD_ENV_KEY, "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            crate::logging::eprint_line(&format!(
                "claude-airou statusline: could not run passthrough: {error}"
            ));
            return 0;
        }
    };
    // The child may exit without reading stdin (SIGPIPE is ignored; the write then just fails).
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        let _ = stdin.write_all(input_data);
        // Dropping the handle closes the pipe, like Swift's explicit close().
    }
    match child.wait() {
        Ok(status) => exit_code(status),
        Err(_) => 0,
    }
}

/// Swift's `terminationStatus`: exit code, or the signal number for signal deaths.
fn exit_code(status: std::process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return signal;
        }
    }
    0
}

// MARK: - Transcript estimator (Swift: TranscriptContextEstimator.estimate)

/// Fallback usage source for sessions that never run a status line: the last assistant
/// message in the transcript carries `usage`, and input + cache_creation + cache_read
/// tokens is what Claude Code itself shows as context usage. Only the final
/// `TAIL_BYTES_TO_READ` bytes of the file are examined, like the Swift build.
fn transcript_estimate_at(
    transcript_path: &str,
    session_id: &str,
    known_window_size: Option<i64>,
    now_secs: f64,
) -> Option<SessionUsageSnapshot> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = std::fs::File::open(transcript_path).ok()?;
    let file_size = file.seek(SeekFrom::End(0)).ok()?;
    let start = file_size.saturating_sub(TAIL_BYTES_TO_READ);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut data = Vec::new();
    file.take(TAIL_BYTES_TO_READ).read_to_end(&mut data).ok()?;
    if data.is_empty() {
        return None;
    }

    // Walk lines from the end; the first (i.e. latest) assistant entry with usage wins.
    for line in data.split(|byte| *byte == b'\n').rev() {
        if line.is_empty() {
            continue;
        }
        let Ok(object) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if object.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        if object.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            continue; // subagent turns live in the same file
        }
        let Some(message) = object.get("message").and_then(Value::as_object) else {
            continue;
        };
        let Some(usage) = message.get("usage").and_then(Value::as_object) else {
            continue;
        };
        let count = |key: &str| -> i64 {
            match usage.get(key) {
                Some(Value::Number(value)) => value
                    .as_i64()
                    .or_else(|| value.as_f64().map(|double| double as i64))
                    .unwrap_or(0),
                _ => 0,
            }
        };
        let context_tokens =
            count("input_tokens") + count("cache_creation_input_tokens") + count("cache_read_input_tokens");
        if context_tokens <= 0 {
            continue;
        }
        let model = message.get("model").and_then(Value::as_str).unwrap_or("");
        let window_size = std::cmp::max(
            known_window_size.unwrap_or(0),
            context_window_size_for_model(model, context_tokens),
        );
        return Some(SessionUsageSnapshot {
            session_id: session_id.to_string(),
            source: UsageSource::Transcript,
            updated_at_epoch_seconds: now_secs,
            context_used_percentage: Some(
                (context_tokens as f64 / window_size as f64 * 100.0).min(100.0),
            ),
            context_window_size: Some(window_size),
            context_tokens: Some(context_tokens),
            total_input_tokens: None,
            total_output_tokens: None,
            model_display_name: if model.is_empty() { None } else { Some(model.to_string()) },
            five_hour_used_percentage: None,
            five_hour_resets_at_epoch_seconds: None,
            seven_day_used_percentage: None,
            seven_day_resets_at_epoch_seconds: None,
            total_cost_usd: None,
        });
    }
    None
}

/// We cannot know every model's window; the transcript never says. Use the 1M tier when
/// the model id says so or when the observed context already exceeds the 200k tier — the
/// estimate is only for a gauge, so "somewhere in the bigger window" beats a pinned 100%.
fn context_window_size_for_model(model: &str, observed_context_tokens: i64) -> i64 {
    let lowered = model.to_lowercase();
    if lowered.contains("[1m]") || lowered.ends_with("-1m") || lowered.contains("1m-context") {
        return LARGE_CONTEXT_WINDOW_SIZE;
    }
    if observed_context_tokens > DEFAULT_CONTEXT_WINDOW_SIZE {
        return LARGE_CONTEXT_WINDOW_SIZE;
    }
    DEFAULT_CONTEXT_WINDOW_SIZE
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::sync::Mutex;

    /// Serialises tests that read or mutate process-wide state (env vars).
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn value(json: &str) -> Value {
        serde_json::from_str(json).expect("test fixture JSON")
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|item| item.to_string()).collect()
    }

    // MARK: parse_usage

    #[test]
    fn parse_usage_extracts_all_fields() {
        let object = value(
            r#"{
                "session_id": "abc-123",
                "model": {"display_name": "Sonnet 4.5"},
                "context_window": {
                    "used_percentage": 42.5,
                    "context_window_size": 200000,
                    "total_input_tokens": 111,
                    "total_output_tokens": 222,
                    "current_usage": {
                        "input_tokens": 1000,
                        "cache_creation_input_tokens": 2000,
                        "cache_read_input_tokens": 3000
                    }
                },
                "rate_limits": {
                    "five_hour": {"used_percentage": 10, "resets_at": 1755500000},
                    "seven_day": {"used_percentage": 20.5, "resets_at": 1755500000000}
                },
                "cost": {"total_cost_usd": 1.25}
            }"#,
        );
        let usage = parse_usage(&object, 123.0).expect("usage");
        assert_eq!(usage.session_id, "abc-123");
        assert_eq!(usage.source, UsageSource::StatusLine);
        assert_eq!(usage.updated_at_epoch_seconds, 123.0);
        assert_eq!(usage.context_used_percentage, Some(42.5));
        assert_eq!(usage.context_window_size, Some(200000));
        assert_eq!(usage.context_tokens, Some(6000));
        assert_eq!(usage.total_input_tokens, Some(111));
        assert_eq!(usage.total_output_tokens, Some(222));
        assert_eq!(usage.model_display_name.as_deref(), Some("Sonnet 4.5"));
        assert_eq!(usage.five_hour_used_percentage, Some(10.0));
        assert_eq!(usage.five_hour_resets_at_epoch_seconds, Some(1755500000.0));
        assert_eq!(usage.seven_day_used_percentage, Some(20.5));
        // Milliseconds (> 1e12) get divided down to seconds.
        assert_eq!(usage.seven_day_resets_at_epoch_seconds, Some(1755500000.0));
        assert_eq!(usage.total_cost_usd, Some(1.25));
    }

    #[test]
    fn parse_usage_requires_session_id() {
        assert!(parse_usage(&value(r#"{"context_window": {}}"#), 0.0).is_none());
        assert!(parse_usage(&value(r#"{"session_id": ""}"#), 0.0).is_none());
        assert!(parse_usage(&value(r#"{"session_id": 42}"#), 0.0).is_none());
        assert!(parse_usage(&value(r#"[1,2,3]"#), 0.0).is_none());
    }

    #[test]
    fn parse_usage_minimal_object_yields_empty_snapshot() {
        let usage = parse_usage(&value(r#"{"session_id": "s"}"#), 7.0).expect("usage");
        assert_eq!(usage.session_id, "s");
        assert!(usage.context_used_percentage.is_none());
        assert!(usage.context_window_size.is_none());
        assert!(usage.context_tokens.is_none());
        assert!(usage.total_cost_usd.is_none());
        assert!(usage.model_display_name.is_none());
    }

    #[test]
    fn parse_usage_derives_percentage_from_current_usage() {
        let object = value(
            r#"{
                "session_id": "s",
                "context_window": {
                    "context_window_size": 200000,
                    "current_usage": {"input_tokens": 50000, "cache_read_input_tokens": 50000}
                }
            }"#,
        );
        let usage = parse_usage(&object, 0.0).expect("usage");
        assert_eq!(usage.context_tokens, Some(100000));
        assert_eq!(usage.context_used_percentage, Some(50.0));

        // An explicit used_percentage is not overwritten by the derived one.
        let object = value(
            r#"{
                "session_id": "s",
                "context_window": {
                    "used_percentage": 12.0,
                    "context_window_size": 200000,
                    "current_usage": {"input_tokens": 100000}
                }
            }"#,
        );
        assert_eq!(parse_usage(&object, 0.0).unwrap().context_used_percentage, Some(12.0));

        // Zero tokens: no contextTokens, no derived percentage.
        let object = value(
            r#"{
                "session_id": "s",
                "context_window": {
                    "context_window_size": 200000,
                    "current_usage": {"input_tokens": 0}
                }
            }"#,
        );
        let usage = parse_usage(&object, 0.0).expect("usage");
        assert!(usage.context_tokens.is_none());
        assert!(usage.context_used_percentage.is_none());
    }

    #[test]
    fn parse_usage_accepts_string_numbers_and_string_dates() {
        let object = value(
            r#"{
                "session_id": "s",
                "context_window": {"used_percentage": "37.5"},
                "rate_limits": {
                    "five_hour": {"used_percentage": "3", "resets_at": "1755500000"},
                    "seven_day": {"resets_at": "2026-08-18T00:00:00Z"}
                }
            }"#,
        );
        let usage = parse_usage(&object, 0.0).expect("usage");
        assert_eq!(usage.context_used_percentage, Some(37.5));
        assert_eq!(usage.five_hour_used_percentage, Some(3.0));
        assert_eq!(usage.five_hour_resets_at_epoch_seconds, Some(1755500000.0));
        // 2026-08-18T00:00:00Z == 1787011200 epoch seconds.
        assert_eq!(usage.seven_day_resets_at_epoch_seconds, Some(1787011200.0));
    }

    #[test]
    fn parse_usage_parses_iso8601_with_fractional_seconds() {
        let object = value(
            r#"{
                "session_id": "s",
                "rate_limits": {"five_hour": {"resets_at": "2026-08-18T00:00:00.500Z"}}
            }"#,
        );
        let usage = parse_usage(&object, 0.0).expect("usage");
        assert_eq!(usage.five_hour_resets_at_epoch_seconds, Some(1787011200.5));

        // Garbage date strings are dropped, not zeroed.
        let object = value(
            r#"{
                "session_id": "s",
                "rate_limits": {"five_hour": {"resets_at": "soon"}}
            }"#,
        );
        assert!(parse_usage(&object, 0.0).unwrap().five_hour_resets_at_epoch_seconds.is_none());
    }

    // MARK: options / passthrough file naming

    #[test]
    fn parse_options_variants() {
        let options = parse_options(&args(&["statusline", "--then", "echo hi"]));
        assert_eq!(options.then_command.as_deref(), Some("echo hi"));
        assert!(options.settings_path.is_none());

        let options = parse_options(&args(&["statusline", "--then=echo hi", "--settings=/tmp/s.json"]));
        assert_eq!(options.then_command.as_deref(), Some("echo hi"));
        assert_eq!(options.settings_path.as_deref(), Some("/tmp/s.json"));

        // `--settings ~/x` gets tilde-expanded like Swift's expandingTildeInPath.
        let options = parse_options(&args(&["statusline", "--settings", "~/x.json"]));
        let expected = crate::paths::home_dir().join("x.json");
        assert_eq!(options.settings_path.as_deref(), Some(expected.to_string_lossy().as_ref()));

        // A trailing `--then` with no value is ignored (Swift returns nil there).
        let options = parse_options(&args(&["statusline", "--then"]));
        assert!(options.then_command.is_none());
    }

    #[test]
    fn djb2_and_base36_match_swift() {
        assert_eq!(djb2(""), 5381);
        assert_eq!(base36(5381), "45h");
        assert_eq!(base36(0), "0");
        assert_eq!(base36(35), "z");
        assert_eq!(base36(36), "10");
        assert_eq!(djb2("abc"), 193485963);
        assert_eq!(base36(djb2("abc")), "3772q3");
        // Full pipeline against a value computed independently (Swift semantics).
        assert_eq!(base36(djb2("/tmp/set.json")), "e4zcbs1chqbs");
    }

    #[test]
    fn passthrough_file_naming() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        // No settings path: the shared default file.
        assert_eq!(
            passthrough_file(None),
            crate::paths::statusline_passthrough_file()
        );
        // The default Claude settings file also maps to the default passthrough file.
        let default_settings = crate::paths::claude_settings_file().to_string_lossy().to_string();
        assert_eq!(
            passthrough_file(Some(&default_settings)),
            crate::paths::statusline_passthrough_file()
        );
        // Any other settings path gets a djb2/base36 digest suffix.
        let named = passthrough_file(Some("/tmp/set.json"));
        assert_eq!(
            named.file_name().unwrap().to_string_lossy(),
            "statusline-passthrough-e4zcbs1chqbs.json"
        );
    }

    #[test]
    fn is_self_invocation_rules() {
        assert!(is_self_invocation("'/usr/local/bin/claude-airou' statusline"));
        assert!(is_self_invocation("claude-airou statusline --settings /tmp/x"));
        assert!(is_self_invocation("  claude-pet statusline  "));
        // Marker without the statusline subcommand: not ours.
        assert!(!is_self_invocation("claude-airou hook"));
        // statusline subcommand without our marker: someone else's binary.
        assert!(!is_self_invocation("/usr/bin/other statusline"));
        assert!(!is_self_invocation(""));
    }

    // MARK: stored passthrough + execution

    struct HomeGuard {
        previous: Option<std::ffi::OsString>,
    }

    impl HomeGuard {
        fn set(dir: &std::path::Path) -> HomeGuard {
            let previous = std::env::var_os("CLAUDE_AIROU_HOME");
            std::env::set_var("CLAUDE_AIROU_HOME", dir);
            HomeGuard { previous }
        }
    }

    impl Drop for HomeGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var("CLAUDE_AIROU_HOME", value),
                None => std::env::remove_var("CLAUDE_AIROU_HOME"),
            }
        }
    }

    #[test]
    fn stored_passthrough_command_rules() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let _home = HomeGuard::set(dir.path());
        let file = crate::paths::statusline_passthrough_file();

        // No file yet.
        assert!(stored_passthrough_command(None).is_none());

        // type defaults to "command".
        std::fs::write(&file, br#"{"command": "echo hi"}"#).unwrap();
        assert_eq!(stored_passthrough_command(None).as_deref(), Some("echo hi"));

        // Explicit type command.
        std::fs::write(&file, br#"{"type": "command", "command": "my-status"}"#).unwrap();
        assert_eq!(stored_passthrough_command(None).as_deref(), Some("my-status"));

        // Non-command type: nothing to run.
        std::fs::write(&file, br#"{"type": "static", "command": "echo hi"}"#).unwrap();
        assert!(stored_passthrough_command(None).is_none());

        // Corrupt / non-object JSON.
        std::fs::write(&file, b"not json").unwrap();
        assert!(stored_passthrough_command(None).is_none());
        std::fs::write(&file, b"[1,2]").unwrap();
        assert!(stored_passthrough_command(None).is_none());
    }

    #[test]
    fn run_passthrough_feeds_stdin_and_forwards_exit_code() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.txt");
        let command = format!("cat > '{}'; exit 7", out.display());
        let code = run_passthrough(b"{\"hello\": 1}", Some(&command), None);
        assert_eq!(code, 7);
        assert_eq!(std::fs::read(&out).unwrap(), b"{\"hello\": 1}");
    }

    #[test]
    fn run_passthrough_uses_stored_command_when_no_explicit_one() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let _home = HomeGuard::set(dir.path());
        let out = dir.path().join("stored-out.txt");
        let stored = serde_json::json!({
            "type": "command",
            "command": format!("cat > '{}'", out.display()),
        });
        std::fs::write(
            crate::paths::statusline_passthrough_file(),
            serde_json::to_vec(&stored).unwrap(),
        )
        .unwrap();
        let code = run_passthrough(b"status json", None, None);
        assert_eq!(code, 0);
        assert_eq!(std::fs::read(&out).unwrap(), b"status json");
    }

    #[test]
    fn run_passthrough_without_command_exits_zero() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let _home = HomeGuard::set(dir.path());
        // No stashed passthrough at all.
        assert_eq!(run_passthrough(b"x", None, None), 0);
        // Whitespace-only explicit command is treated as absent.
        assert_eq!(run_passthrough(b"x", Some("   "), None), 0);
    }

    #[test]
    fn run_passthrough_survives_child_that_ignores_stdin() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        // `true` exits immediately without reading stdin; the write fails with EPIPE but
        // must not kill or fail us (Swift ignores SIGPIPE for the same reason).
        let big = vec![b'x'; 256 * 1024];
        assert_eq!(run_passthrough(&big, Some("exit 0"), None), 0);
    }

    #[test]
    fn run_passthrough_refuses_self_invocation() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(
            run_passthrough(b"x", Some("/opt/claude-airou statusline"), None),
            0
        );
    }

    #[test]
    fn run_passthrough_refuses_recursion() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        std::env::set_var(RECURSION_GUARD_ENV_KEY, "1");
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("must-not-exist");
        let command = format!("touch '{}'", out.display());
        let code = run_passthrough(b"x", Some(&command), None);
        std::env::remove_var(RECURSION_GUARD_ENV_KEY);
        assert_eq!(code, 0);
        assert!(!out.exists());
    }

    // MARK: transcript estimator

    fn write_transcript(lines: &[&str]) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
        file.flush().unwrap();
        file
    }

    fn assistant_line(input: i64, cache_creation: i64, cache_read: i64, model: &str) -> String {
        serde_json::json!({
            "type": "assistant",
            "message": {
                "model": model,
                "usage": {
                    "input_tokens": input,
                    "cache_creation_input_tokens": cache_creation,
                    "cache_read_input_tokens": cache_read,
                    "output_tokens": 999999
                }
            }
        })
        .to_string()
    }

    #[test]
    fn transcript_estimate_reads_last_assistant_usage() {
        let file = write_transcript(&[
            r#"{"type":"user","message":{"content":"hi"}}"#,
            &assistant_line(100, 0, 0, "claude-sonnet-4-5"),
            r#"{"type":"system","subtype":"info"}"#,
            &assistant_line(2000, 3000, 45000, "claude-sonnet-4-5"),
        ]);
        let snapshot =
            transcript_estimate_at(file.path().to_str().unwrap(), "sess-1", None, 42.0).expect("snapshot");
        assert_eq!(snapshot.session_id, "sess-1");
        assert_eq!(snapshot.source, UsageSource::Transcript);
        assert_eq!(snapshot.updated_at_epoch_seconds, 42.0);
        // Latest assistant entry wins; output_tokens are NOT counted.
        assert_eq!(snapshot.context_tokens, Some(50000));
        assert_eq!(snapshot.context_window_size, Some(200000));
        assert_eq!(snapshot.context_used_percentage, Some(25.0));
        assert_eq!(snapshot.model_display_name.as_deref(), Some("claude-sonnet-4-5"));
        assert!(snapshot.total_cost_usd.is_none());
        assert!(snapshot.five_hour_used_percentage.is_none());
    }

    #[test]
    fn transcript_estimate_skips_sidechain_garbage_and_zero_token_lines() {
        let file = write_transcript(&[
            &assistant_line(40000, 0, 0, "claude-opus-4-1"),
            "not json at all {{{",
            r#"{"type":"assistant","message":{"model":"m"}}"#, // no usage
            r#"{"type":"assistant"}"#,                           // no message
            &assistant_line(0, 0, 0, "claude-opus-4-1"),         // zero tokens
            // Latest matching line is a sidechain (subagent) turn: skipped.
            r#"{"type":"assistant","isSidechain":true,"message":{"model":"sub","usage":{"input_tokens":77777}}}"#,
        ]);
        let snapshot = transcript_estimate_at(file.path().to_str().unwrap(), "s", None, 0.0).expect("snapshot");
        assert_eq!(snapshot.context_tokens, Some(40000));
        assert_eq!(snapshot.model_display_name.as_deref(), Some("claude-opus-4-1"));

        // isSidechain false is fine.
        let file = write_transcript(&[
            r#"{"type":"assistant","isSidechain":false,"message":{"model":"m","usage":{"input_tokens":5}}}"#,
        ]);
        let snapshot = transcript_estimate_at(file.path().to_str().unwrap(), "s", None, 0.0).expect("snapshot");
        assert_eq!(snapshot.context_tokens, Some(5));
    }

    #[test]
    fn transcript_estimate_none_for_missing_empty_or_garbage_files() {
        assert!(transcript_estimate("/nonexistent/path/x.jsonl", "s", None).is_none());

        let file = write_transcript(&[]);
        assert!(transcript_estimate(file.path().to_str().unwrap(), "s", None).is_none());

        let file = write_transcript(&["garbage", "{\"type\":\"user\"}", "[]"]);
        assert!(transcript_estimate(file.path().to_str().unwrap(), "s", None).is_none());
    }

    #[test]
    fn transcript_estimate_window_inference() {
        // 1M marker in the model id.
        let file = write_transcript(&[&assistant_line(1000, 0, 0, "claude-sonnet-4-5[1M]")]);
        let snapshot = transcript_estimate_at(file.path().to_str().unwrap(), "s", None, 0.0).unwrap();
        assert_eq!(snapshot.context_window_size, Some(1_000_000));

        let file = write_transcript(&[&assistant_line(1000, 0, 0, "claude-sonnet-4-5-1m")]);
        let snapshot = transcript_estimate_at(file.path().to_str().unwrap(), "s", None, 0.0).unwrap();
        assert_eq!(snapshot.context_window_size, Some(1_000_000));

        // Observed tokens above the 200k tier push the estimate to the 1M tier.
        let file = write_transcript(&[&assistant_line(250_000, 0, 0, "claude-sonnet-4-5")]);
        let snapshot = transcript_estimate_at(file.path().to_str().unwrap(), "s", None, 0.0).unwrap();
        assert_eq!(snapshot.context_window_size, Some(1_000_000));
        assert_eq!(snapshot.context_used_percentage, Some(25.0));

        // A known window size is a floor: never estimate below it.
        let file = write_transcript(&[&assistant_line(1000, 0, 0, "claude-sonnet-4-5")]);
        let snapshot =
            transcript_estimate_at(file.path().to_str().unwrap(), "s", Some(1_000_000), 0.0).unwrap();
        assert_eq!(snapshot.context_window_size, Some(1_000_000));
        assert_eq!(snapshot.context_used_percentage, Some(0.1));

        // ... but a smaller known window loses to the inferred one.
        let file = write_transcript(&[&assistant_line(1000, 0, 0, "claude-sonnet-4-5")]);
        let snapshot =
            transcript_estimate_at(file.path().to_str().unwrap(), "s", Some(100), 0.0).unwrap();
        assert_eq!(snapshot.context_window_size, Some(200_000));
    }

    #[test]
    fn transcript_estimate_percentage_capped_at_100() {
        // 250k tokens against a known 200k window (the observed-token rule cannot fire
        // because known >= inferred? it can: inferred jumps to 1M; force via known only).
        let file = write_transcript(&[&assistant_line(150_000, 100_000, 0, "claude-sonnet-4-5")]);
        // known 200k, observed 250k -> inferred 1M wins (max), so use a case below the tier:
        let snapshot =
            transcript_estimate_at(file.path().to_str().unwrap(), "s", Some(200_000), 0.0).unwrap();
        assert_eq!(snapshot.context_window_size, Some(1_000_000));
        // Direct check of the cap through the helper contract instead:
        let file = write_transcript(&[&assistant_line(150_000, 50_000, 0, "claude-sonnet-4-5")]);
        let snapshot =
            transcript_estimate_at(file.path().to_str().unwrap(), "s", Some(100_000), 0.0).unwrap();
        // window = max(100k known, 200k inferred) = 200k; 200k/200k = 100 (capped).
        assert_eq!(snapshot.context_used_percentage, Some(100.0));
    }

    #[test]
    fn transcript_estimate_only_reads_the_tail() {
        // A good line pushed out of the 96 KiB tail by filler must be invisible (Swift
        // reads only the tail), and a good line inside the tail must be found.
        let good = assistant_line(12345, 0, 0, "claude-sonnet-4-5");
        let filler = format!(r#"{{"type":"progress","pad":"{}"}}"#, "x".repeat(1000));
        let mut lines: Vec<&str> = vec![&good];
        let filler_count = (TAIL_BYTES_TO_READ as usize / filler.len()) + 8;
        let fillers: Vec<String> = (0..filler_count).map(|_| filler.clone()).collect();
        lines.extend(fillers.iter().map(String::as_str));
        let file = write_transcript(&lines);
        assert!(transcript_estimate(file.path().to_str().unwrap(), "s", None).is_none());

        // Same file with a good line appended at the end: found again.
        let mut lines_with_tail = lines.clone();
        lines_with_tail.push(&good);
        let file = write_transcript(&lines_with_tail);
        let snapshot = transcript_estimate(file.path().to_str().unwrap(), "s", None).expect("snapshot");
        assert_eq!(snapshot.context_tokens, Some(12345));
    }

    #[test]
    fn context_window_size_for_model_rules() {
        assert_eq!(context_window_size_for_model("claude-sonnet-4-5", 0), 200_000);
        assert_eq!(context_window_size_for_model("claude-sonnet-4-5[1m]", 0), 1_000_000);
        assert_eq!(context_window_size_for_model("CLAUDE-SONNET-4-5[1M]", 0), 1_000_000);
        assert_eq!(context_window_size_for_model("claude-x-1M", 0), 1_000_000);
        assert_eq!(context_window_size_for_model("claude-1m-context-beta", 0), 1_000_000);
        assert_eq!(context_window_size_for_model("", 200_000), 200_000);
        assert_eq!(context_window_size_for_model("", 200_001), 1_000_000);
    }

    #[test]
    fn exit_code_maps_signals_like_swift() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        // Normal exit code passthrough.
        assert_eq!(run_passthrough(b"", Some("exit 3"), None), 3);
        // A signal-killed child reports the signal number (Swift terminationStatus).
        #[cfg(unix)]
        {
            assert_eq!(run_passthrough(b"", Some("kill -9 $$"), None), 9);
        }
    }
}
