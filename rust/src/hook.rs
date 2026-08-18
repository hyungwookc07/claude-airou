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

pub fn run() -> i32 {
    todo!("port HookCommand.run from Sources/ClaudeAirou/Hook/HookCommand.swift")
}
