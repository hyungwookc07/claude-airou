//! `claude-airou mcp` — stdio MCP server so Claude chat (the Claude desktop app) can drive
//! the pet. Port of `MCP/MCPServer.swift`; behaviour must match:
//!
//! - Newline-delimited JSON-RPC 2.0 on stdin/stdout; stdout carries protocol messages only,
//!   diagnostics go to `paths::mcp_log_file()` (append, truncate past 512 KiB).
//! - Supported protocol versions {"2024-11-05","2025-03-26","2025-06-18"}; echo the client's
//!   if supported, else answer "2025-06-18".
//! - Session id `claude-chat-<pid>`; label from clientInfo.name ("claude"+"code" → "Claude
//!   Code", contains "claude" → "Claude Chat", empty → "Claude Chat", else verbatim);
//!   the label is written as the snapshot's `cwd`.
//! - initialize → write hello ("Hi! Ready when you are"), respond with capabilities
//!   {tools:{}}, serverInfo {name:"claude-airou", version}, and the same `instructions`
//!   text as the Swift server.
//! - tools/list → `mcp_tools::descriptors()`; tools/call → `mcp_tools::call(...)`; after a
//!   successful non-pet_status call write thinking ("Thinking…"). Unknown tool → error
//!   -32602; unknown method with id → -32601; parse error → -32700 (id null);
//!   notifications/* ignored; ping → {}.
//! - Idle watchdog: every 30 s, a busy state older than 180 s is reset to idle
//!   ("mcp:idle-watchdog").
//! - stdin EOF / SIGTERM / SIGINT / SIGHUP → remove the session file, exit 0.
//!
//! Threading: blocking stdin loop + one watchdog thread; guard label/last-write with a Mutex.

use crate::model::PetState;
use crate::state_store::StateStore;
use std::sync::{Arc, Mutex};

pub struct ServerState {
    pub store: StateStore,
    pub session_id: String,
    pub session_label: String,
    pub last_written_state: Option<PetState>,
    pub last_write_epoch_secs: f64,
}

pub type SharedServerState = Arc<Mutex<ServerState>>;

/// Writes the session snapshot the overlay reads (same shape the Swift MCP server writes:
/// cwd = label, tool_name = None, last_event_name = `event`), updates last-write bookkeeping
/// and logs "<event> -> <state> \"<message>\"".
pub fn write_state(shared: &SharedServerState, state: PetState, message: &str, event: &str) {
    let _ = (shared, state, message, event);
    todo!()
}

pub fn run() -> i32 {
    todo!("port MCPServer from Sources/ClaudeAirou/MCP/MCPServer.swift")
}
