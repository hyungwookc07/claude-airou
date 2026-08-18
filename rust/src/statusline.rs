//! `claude-airou statusline` (usage recording + passthrough) and the transcript-based
//! context estimator. Ports of `Hook/StatusLineCommand.swift` and
//! `Hook/TranscriptContextEstimator.swift` — read those files for the exact JSON fields,
//! passthrough semantics (run the user's original status line command with the same stdin,
//! forward its stdout/exit code) and estimation rules (last assistant message's token usage
//! in the session's transcript JSONL; cache_read+cache_creation+input(+output) tokens;
//! known window size preferred, else infer from model; source = "transcript").

use crate::model::SessionUsageSnapshot;

/// Hook events on which the estimate is refreshed (Swift:
/// `TranscriptContextEstimator.refreshingEventNames`).
pub const REFRESHING_EVENT_NAMES: [&str; 4] =
    ["PostToolUse", "PostToolBatch", "Stop", "UserPromptSubmit"];

/// Entry point for `claude-airou statusline`. Reads the status line JSON on stdin, records
/// a `SessionUsageSnapshot` (source status_line) via `StateStore::merge_usage`, then execs
/// the stashed passthrough command (paths::statusline_passthrough_file()) with the same
/// stdin bytes and mirrors its stdout / exit code. Never fails the status line: on any
/// internal error still run the passthrough (or exit 0).
pub fn run(raw_args: &[String]) -> i32 {
    let _ = raw_args;
    todo!("port StatusLineCommand.run from Sources/ClaudeAirou/Hook/StatusLineCommand.swift")
}

/// Estimate context usage from the transcript JSONL; `None` when nothing usable is found.
pub fn transcript_estimate(
    transcript_path: &str,
    session_id: &str,
    known_window_size: Option<i64>,
) -> Option<SessionUsageSnapshot> {
    let _ = (transcript_path, session_id, known_window_size);
    todo!("port TranscriptContextEstimator.estimate from Sources/ClaudeAirou/Hook/TranscriptContextEstimator.swift")
}
