//! Entry point for `claude-airou hook`. Port of `Hook/HookCommand.swift`.
//!
//! Contract with Claude Code (identical to the Swift binary):
//!  - never write to stdout (some events' hook stdout is injected into the model context)
//!  - always exit 0 (a non-zero exit surfaces as a hook error inside Claude Code)
//!  - be fast (runs synchronously before/after every tool call)
//!
//! Flow: read stdin JSON → refresh the transcript usage estimate when relevant
//! (`statusline::REFRESHING_EVENT_NAMES`, not a subagent event, transcript_path present;
//! merge via `StateStore::merge_usage`) → `hook_mapper::map` → on Update run
//! `hook_mapper::resolve` against the existing snapshot and write/keep → on RemoveSession
//! delete the state file. Log one line per event to `paths::hook_log_file()` in the same
//! shapes as Swift ("<event> <session> -> <state> \"<msg>\"", "… kept (<reason>)",
//! "… removed", "… ignored", "unparseable input (N bytes)").

use crate::hook_mapper::{self, HookInput, MappingResult, Resolution};
use crate::state_store::StateStore;
use std::io::{IsTerminal, Read};
use std::path::Path;

pub fn run() -> i32 {
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        crate::logging::eprint_line(
            "claude-airou hook: expects Claude Code hook JSON on stdin (see `claude-airou install-hooks`).",
        );
        return 0;
    }

    let mut input_data = Vec::new();
    // Best-effort read: whatever arrived before an error still gets a parse attempt,
    // matching Foundation's readDataToEndOfFile.
    let _ = stdin.lock().read_to_end(&mut input_data);

    let store = StateStore::default();
    process(&input_data, &store, &crate::paths::hook_log_file());
    0
}

/// Everything after stdin has been read, factored out so tests can drive it with an
/// in-memory payload, a temp state directory and a temp log file.
fn process(input_data: &[u8], store: &StateStore, log_path: &Path) {
    let Some(input) = HookInput::parse(input_data) else {
        crate::logging::append(log_path, &format!("unparseable input ({} bytes)", input_data.len()));
        return;
    };

    refresh_usage_estimate_if_relevant(&input, store);

    match hook_mapper::map(&input) {
        MappingResult::Ignore => {
            crate::logging::append(
                log_path,
                &format!("{} {} ignored", input.hook_event_name(), input.session_id()),
            );
        }

        MappingResult::RemoveSession => {
            store.remove(input.session_id());
            crate::logging::append(
                log_path,
                &format!("{} {} removed", input.hook_event_name(), input.session_id()),
            );
        }

        MappingResult::Update { state, message, tool_name } => {
            let existing = store.read(input.session_id());
            let resolution = hook_mapper::resolve(
                existing.as_ref(),
                &input,
                state,
                &message,
                tool_name.as_deref(),
                crate::model::now_epoch_secs(),
            );
            match resolution {
                Resolution::RosterOnly(snapshot) => {
                    let agent_count = snapshot.active_agents.len();
                    match store.write(&snapshot) {
                        Ok(()) => crate::logging::append(
                            log_path,
                            &format!(
                                "{} {} roster ({agent_count} agent(s) working)",
                                input.hook_event_name(),
                                input.session_id()
                            ),
                        ),
                        Err(error) => crate::logging::append(
                            log_path,
                            &format!("{} {} roster write failed: {error}", input.hook_event_name(), input.session_id()),
                        ),
                    }
                }
                Resolution::Keep(reason) => {
                    crate::logging::append(
                        log_path,
                        &format!("{} {} kept ({reason})", input.hook_event_name(), input.session_id()),
                    );
                }
                Resolution::Write(snapshot) => match store.write(&snapshot) {
                    Ok(()) => {
                        let agent_suffix = input
                            .agent_id()
                            .map(|id| format!(" [agent {}]", id.chars().take(8).collect::<String>()))
                            .unwrap_or_default();
                        crate::logging::append(
                            log_path,
                            &format!(
                                "{}{agent_suffix} {} -> {} \"{message}\"",
                                input.hook_event_name(),
                                input.session_id(),
                                state.raw()
                            ),
                        );
                    }
                    Err(error) => {
                        crate::logging::append(
                            log_path,
                            &format!("{} {} write failed: {error}", input.hook_event_name(), input.session_id()),
                        );
                    }
                },
            }
        }
    }
}

/// Fallback context gauge for sessions without a status line: estimate from the transcript.
/// `merge_usage` refuses to overwrite a recent status-line reading.
///
/// `statusline::transcript_estimate` is wrapped in `catch_unwind` permanently: the hook must
/// never fail, no matter what the estimator does (today it may still be a `todo!()` stub).
fn refresh_usage_estimate_if_relevant(input: &HookInput, store: &StateStore) {
    if !crate::statusline::REFRESHING_EVENT_NAMES.contains(&input.hook_event_name()) {
        return;
    }
    if input.is_subagent_event() {
        return;
    }
    let Some(transcript_path) = input.transcript_path() else {
        return;
    };
    let known = store
        .read_usage(input.session_id())
        .and_then(|usage| usage.context_window_size);
    let estimate = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::statusline::transcript_estimate(transcript_path, input.session_id(), known)
    }));
    if let Ok(Some(estimate)) = estimate {
        let _ = store.merge_usage(&estimate);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{now_epoch_secs, PetState, SessionSnapshot};
    use std::path::PathBuf;

    struct Fixture {
        _dir: tempfile::TempDir,
        store: StateStore,
        log_path: PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path().join("state"));
        let log_path = dir.path().join("hook.log");
        Fixture { store, log_path, _dir: dir }
    }

    fn log_contents(fixture: &Fixture) -> String {
        std::fs::read_to_string(&fixture.log_path).unwrap_or_default()
    }

    #[test]
    fn unparseable_input_logs_byte_count() {
        let fixture = fixture();
        process(b"not json at all", &fixture.store, &fixture.log_path);
        assert!(log_contents(&fixture).contains("unparseable input (15 bytes)"));
        assert!(fixture.store.load_all().is_empty());
    }

    #[test]
    fn session_start_writes_snapshot_and_logs() {
        let fixture = fixture();
        let payload = br#"{"hook_event_name":"SessionStart","session_id":"sess-1","cwd":"/tmp/project"}"#;
        process(payload, &fixture.store, &fixture.log_path);
        let snapshot = fixture.store.read("sess-1").expect("snapshot written");
        assert_eq!(snapshot.state, PetState::Hello);
        assert_eq!(snapshot.message, "Hi! Ready when you are");
        assert_eq!(snapshot.cwd, "/tmp/project");
        assert!(log_contents(&fixture).contains("SessionStart sess-1 -> hello \"Hi! Ready when you are\""));
    }

    #[test]
    fn session_end_removes_snapshot_and_logs() {
        let fixture = fixture();
        process(
            br#"{"hook_event_name":"SessionStart","session_id":"sess-2","cwd":"/tmp"}"#,
            &fixture.store,
            &fixture.log_path,
        );
        assert!(fixture.store.read("sess-2").is_some());
        process(
            br#"{"hook_event_name":"SessionEnd","session_id":"sess-2","cwd":"/tmp","reason":"exit"}"#,
            &fixture.store,
            &fixture.log_path,
        );
        assert!(fixture.store.read("sess-2").is_none());
        assert!(log_contents(&fixture).contains("SessionEnd sess-2 removed"));
    }

    #[test]
    fn unknown_event_is_ignored_and_logged() {
        let fixture = fixture();
        process(
            br#"{"hook_event_name":"SomethingNew","session_id":"sess-3","cwd":"/tmp"}"#,
            &fixture.store,
            &fixture.log_path,
        );
        assert!(fixture.store.read("sess-3").is_none());
        assert!(log_contents(&fixture).contains("SomethingNew sess-3 ignored"));
    }

    #[test]
    fn sibling_tool_completion_is_kept_and_logged() {
        let fixture = fixture();
        let blocked = SessionSnapshot {
            session_id: "sess-4".into(),
            cwd: "/tmp/project".into(),
            state: PetState::WaitingApproval,
            message: "Approve? Running: rm".into(),
            last_event_name: "PermissionRequest".into(),
            tool_name: Some("Bash".into()),
            updated_at_epoch_seconds: now_epoch_secs(),
            pending_tool_use_id: Some("toolu_pending".into()),
            active_agents: Vec::new(),
        };
        fixture.store.write(&blocked).unwrap();
        process(
            br#"{"hook_event_name":"PostToolUse","session_id":"sess-4","cwd":"/tmp/project","tool_name":"Read","tool_use_id":"toolu_sibling"}"#,
            &fixture.store,
            &fixture.log_path,
        );
        // Snapshot untouched.
        let snapshot = fixture.store.read("sess-4").unwrap();
        assert_eq!(snapshot.state, PetState::WaitingApproval);
        assert_eq!(snapshot.pending_tool_use_id.as_deref(), Some("toolu_pending"));
        assert!(log_contents(&fixture).contains(
            "PostToolUse sess-4 kept (sibling tool toolu_sibling finished while waiting on toolu_pending)"
        ));
    }

    #[test]
    fn subagent_write_logs_agent_suffix_with_8_char_prefix() {
        let fixture = fixture();
        process(
            br#"{"hook_event_name":"PreToolUse","session_id":"sess-5","cwd":"/tmp","agent_id":"agent-1234567890","tool_name":"Read","tool_input":{"file_path":"/a/b.rs"}}"#,
            &fixture.store,
            &fixture.log_path,
        );
        assert!(log_contents(&fixture).contains("PreToolUse [agent agent-12] sess-5 -> working \"Reading b.rs\""));
    }

    #[test]
    fn permission_request_stores_pending_tool_use_id() {
        let fixture = fixture();
        process(
            br#"{"hook_event_name":"PermissionRequest","session_id":"sess-6","cwd":"/tmp","tool_name":"Bash","tool_input":{"command":"ls"},"tool_use_id":"toolu_9"}"#,
            &fixture.store,
            &fixture.log_path,
        );
        let snapshot = fixture.store.read("sess-6").unwrap();
        assert_eq!(snapshot.state, PetState::WaitingApproval);
        assert_eq!(snapshot.pending_tool_use_id.as_deref(), Some("toolu_9"));
        assert!(log_contents(&fixture)
            .contains("PermissionRequest sess-6 -> waiting_approval \"Approve? Running: ls\""));
    }

    #[test]
    fn refresh_survives_estimator_panics_and_still_writes_state() {
        // "Stop" is in REFRESHING_EVENT_NAMES and transcript_path is present, so the
        // estimator runs; whether it is still a todo!() stub (panics, caught by
        // catch_unwind) or implemented (returns None for a missing file), the hook must
        // carry on and write the mapped state.
        let fixture = fixture();
        process(
            br#"{"hook_event_name":"Stop","session_id":"sess-7","cwd":"/tmp","transcript_path":"/nonexistent/transcript.jsonl"}"#,
            &fixture.store,
            &fixture.log_path,
        );
        let snapshot = fixture.store.read("sess-7").unwrap();
        assert_eq!(snapshot.state, PetState::Done);
        assert!(log_contents(&fixture).contains("Stop sess-7 -> done \"Done!\""));
    }

    #[test]
    fn subagent_events_skip_usage_refresh() {
        // Same refreshing event but from a subagent: the estimator must not run at all
        // (no panic noise, no usage file), and the mapped state is still written.
        let fixture = fixture();
        process(
            br#"{"hook_event_name":"PostToolUse","session_id":"sess-8","cwd":"/tmp","agent_id":"a1","transcript_path":"/nonexistent/t.jsonl","tool_name":"Read"}"#,
            &fixture.store,
            &fixture.log_path,
        );
        assert!(fixture.store.read_usage("sess-8").is_none());
        let snapshot = fixture.store.read("sess-8").unwrap();
        assert_eq!(snapshot.state, PetState::Thinking);
    }

    #[test]
    fn missing_session_id_falls_back_to_unknown_session() {
        let fixture = fixture();
        process(br#"{"hook_event_name":"Stop"}"#, &fixture.store, &fixture.log_path);
        assert!(fixture.store.read("unknown-session").is_some());
        assert!(log_contents(&fixture).contains("Stop unknown-session -> done \"Done!\""));
    }
}
