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
//! - stdin EOF → remove the session file, exit 0. The Swift server also installs
//!   SIGTERM/SIGINT/SIGHUP handlers for the same cleanup; without a signals crate that part
//!   is best-effort here — see the comment in [`run`].
//!
//! Threading: blocking stdin loop + one watchdog thread; guard label/last-write with a Mutex.

use crate::model::{now_epoch_secs, PetState, SessionSnapshot};
use crate::state_store::StateStore;
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex, MutexGuard};

/// Versions we can speak. An unknown client version is answered with the latest.
const SUPPORTED_PROTOCOL_VERSIONS: [&str; 3] = ["2024-11-05", "2025-03-26", "2025-06-18"];
const LATEST_PROTOCOL_VERSION: &str = "2025-06-18";
/// Chat has no Stop event, so a busy state left behind (Claude never called `pet_status`
/// again) is reset to idle by the watchdog after this long. Attention states keep the
/// longer decay from `PetState::transient_duration_secs`, same as Claude Code sessions.
const BUSY_IDLE_AFTER_SECS: f64 = 3.0 * 60.0;
const WATCHDOG_INTERVAL_SECS: u64 = 30;

/// Same `instructions` text as `MCPServer.swift` (single line — the Swift multiline literal
/// joins its lines with `\`).
const INSTRUCTIONS: &str = "This server controls the user's claude-airou desktop pet — a \
     small pixel companion floating on their screen that mirrors what Claude is doing. Keep \
     it honest: call pet_status(\"thinking\" or \"working\") when you start on a request or \
     a long step, pet_status(\"done\") when you finish, pet_status(\"error\") when something \
     fails, and pet_status(\"needs_input\") when you are waiting for the user's answer. \
     Speech-bubble messages should stay under 60 characters. Use hatch_pet to create or edit \
     custom pets when the user asks for one.";

pub struct ServerState {
    pub store: StateStore,
    pub session_id: String,
    pub session_label: String,
    pub last_written_state: Option<PetState>,
    pub last_write_epoch_secs: f64,
}

pub type SharedServerState = Arc<Mutex<ServerState>>;

/// Locks the shared state; a poisoned mutex (a panic on another thread) still yields the
/// data — the MCP server must never panic itself.
fn lock(shared: &SharedServerState) -> MutexGuard<'_, ServerState> {
    match shared.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Appends one line to `~/.claude-airou/mcp.log` (stdout belongs to the protocol).
fn log(line: &str) {
    crate::logging::append(&crate::paths::mcp_log_file(), line);
}

/// Writes the session snapshot the overlay reads (same shape the Swift MCP server writes:
/// cwd = label, tool_name = None, last_event_name = `event`), updates last-write bookkeeping
/// and logs "<event> -> <state> \"<message>\"".
pub fn write_state(shared: &SharedServerState, state: PetState, message: &str, event: &str) {
    let mut server = lock(shared);
    let now = now_epoch_secs();
    let snapshot = SessionSnapshot {
        session_id: server.session_id.clone(),
        cwd: server.session_label.clone(),
        state,
        message: message.to_string(),
        last_event_name: event.to_string(),
        tool_name: None,
        updated_at_epoch_seconds: now,
        pending_tool_use_id: None,
    };
    match server.store.write(&snapshot) {
        Ok(()) => {
            server.last_written_state = Some(state);
            server.last_write_epoch_secs = now;
            log(&format!("{event} -> {} \"{message}\"", state.raw()));
        }
        Err(error) => log(&format!("{event} write failed: {error}")),
    }
}

/// "claude-ai" (the desktop app) → "Claude Chat"; other MCP clients keep their own name.
fn session_label_for_client_name(client_name: &str) -> String {
    let lowered = client_name.to_lowercase();
    if lowered.is_empty() {
        return "Claude Chat".to_string();
    }
    if lowered.contains("claude") {
        return if lowered.contains("code") {
            "Claude Code"
        } else {
            "Claude Chat"
        }
        .to_string();
    }
    client_name.to_string()
}

// MARK: - JSON-RPC plumbing

fn response(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// One protocol line in, the responses to write out (0, 1, or one per batch element).
/// Mirrors `MCPServer.handleLine`: unparseable → -32700 with id null; a JSON scalar also
/// counts as a parse error (Swift's `JSONSerialization` rejects top-level fragments); an
/// array is a batch when every element is an object, otherwise -32600.
fn handle_line(shared: &SharedServerState, raw_line: &str) -> Vec<Value> {
    let line = raw_line.trim();
    if line.is_empty() {
        return Vec::new();
    }
    let parsed: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(_) => {
            log(&format!("unparseable message ({} bytes)", raw_line.len()));
            return vec![error_response(Value::Null, -32700, "Parse error")];
        }
    };
    match &parsed {
        Value::Object(_) => handle_message(shared, &parsed).into_iter().collect(),
        Value::Array(items) => {
            // Batching was removed from the protocol in 2025-06-18 but old clients may send it.
            if items.iter().all(Value::is_object) {
                items
                    .iter()
                    .filter_map(|item| handle_message(shared, item))
                    .collect()
            } else {
                vec![error_response(Value::Null, -32600, "Invalid request")]
            }
        }
        _ => {
            // Top-level scalar: Foundation's JSONSerialization would have refused to parse it.
            log(&format!("unparseable message ({} bytes)", raw_line.len()));
            vec![error_response(Value::Null, -32700, "Parse error")]
        }
    }
}

fn handle_message(shared: &SharedServerState, message: &Value) -> Option<Value> {
    // A response to a server-initiated request; we never send any, so nothing to match.
    let method = message.get("method")?.as_str()?.to_string();
    let id = message.get("id").cloned();
    let params = message
        .get("params")
        .filter(|params| params.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));

    match method.as_str() {
        "initialize" => handle_initialize(shared, id, &params),
        "ping" => id.map(|id| response(id, json!({}))),
        "tools/list" => id.map(|id| response(id, json!({"tools": crate::mcp_tools::descriptors()}))),
        "tools/call" => handle_tool_call(shared, id, &params),
        name if name.starts_with("notifications/") => {
            // initialized / cancelled / roots changed — nothing to do, but never an error.
            log(&format!("notification {name}"));
            None
        }
        other => {
            log(&format!("unknown method {other}"));
            id.map(|id| error_response(id, -32601, &format!("Method not found: {other}")))
        }
    }
}

// MARK: - Requests

fn handle_initialize(shared: &SharedServerState, id: Option<Value>, params: &Value) -> Option<Value> {
    let client_name = params
        .get("clientInfo")
        .and_then(|info| info.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let label = session_label_for_client_name(client_name);
    {
        let mut server = lock(shared);
        server.session_label = label.clone();
    }
    log(&format!("initialize from \"{client_name}\" → label \"{label}\""));

    let requested_version = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or("");
    let version = if SUPPORTED_PROTOCOL_VERSIONS.contains(&requested_version) {
        requested_version
    } else {
        LATEST_PROTOCOL_VERSION
    };

    write_state(shared, PetState::Hello, "Hi! Ready when you are", "mcp:initialize");

    let id = id?;
    Some(response(
        id,
        json!({
            "protocolVersion": version,
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "claude-airou", "version": crate::cli::VERSION},
            "instructions": INSTRUCTIONS,
        }),
    ))
}

fn handle_tool_call(shared: &SharedServerState, id: Option<Value>, params: &Value) -> Option<Value> {
    let tool_name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let arguments = params
        .get("arguments")
        .filter(|arguments| arguments.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let Some(result) = crate::mcp_tools::call(tool_name, &arguments, shared) else {
        return id.map(|id| error_response(id, -32602, &format!("Unknown tool: {tool_name}")));
    };
    log(&format!(
        "tools/call {tool_name}{}",
        if result.is_error { " (error)" } else { "" }
    ));
    // After any non-status tool Claude reads the result and keeps composing its reply.
    if tool_name != crate::mcp_tools::PET_STATUS_TOOL_NAME && !result.is_error {
        write_state(
            shared,
            PetState::Thinking,
            "Thinking…",
            &format!("mcp:tools/call:{tool_name}"),
        );
    }
    let id = id?;
    Some(response(
        id,
        json!({
            "content": result.content,
            "isError": result.is_error,
        }),
    ))
}

// MARK: - Watchdog & lifecycle

fn idle_watchdog_tick(shared: &SharedServerState) {
    let (state, age) = {
        let server = lock(shared);
        (
            server.last_written_state,
            now_epoch_secs() - server.last_write_epoch_secs,
        )
    };
    let Some(state) = state else { return };
    if state.is_busy() && age > BUSY_IDLE_AFTER_SECS {
        write_state(shared, PetState::Idle, "", "mcp:idle-watchdog");
    }
}

/// The desktop app terminates servers when it quits; remove the session so the
/// overlay does not keep a ghost "Claude Chat" pet around.
fn clean_up_session(shared: &SharedServerState, reason: &str) {
    let server = lock(shared);
    server.store.remove(&server.session_id);
    log(&format!("stopped ({reason})"));
}

/// Writes one JSON object per line and flushes — `serde_json::to_vec` never emits newlines,
/// which is exactly what the stdio transport needs.
fn send(stdout: &mut impl Write, message: &Value) {
    let Ok(mut data) = serde_json::to_vec(message) else {
        log("could not serialize response");
        return;
    };
    data.push(b'\n');
    let _ = stdout.write_all(&data);
    let _ = stdout.flush();
}

pub fn run() -> i32 {
    let shared: SharedServerState = Arc::new(Mutex::new(ServerState {
        store: StateStore::default(),
        // One server process = one chat session (the app launches it once and keeps it running).
        session_id: format!("claude-chat-{}", std::process::id()),
        session_label: "Claude Chat".to_string(),
        last_written_state: None,
        last_write_epoch_secs: now_epoch_secs(),
    }));

    // Idle watchdog: the Swift server uses a DispatchSourceTimer; a detached thread with a
    // 30 s sleep does the same job. It dies with the process.
    let watchdog_shared = Arc::clone(&shared);
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(WATCHDOG_INTERVAL_SECS));
        idle_watchdog_tick(&watchdog_shared);
    });

    // NOTE on signals: the Swift server also installs SIGTERM/SIGINT/SIGHUP handlers that
    // remove the session file before exiting. Without a signals crate (no new dependencies)
    // there is no portable, safe way to do that from std, so this build relies on stdin EOF
    // — the desktop app closes the pipe when it quits — plus the cleanup below. If the
    // process is killed by a signal instead, the leftover state file is covered by the
    // overlay's transient-state decay and the state store's stale-file pruning.
    log(&format!(
        "started (session claude-chat-{pid}, pid {pid})",
        pid = std::process::id()
    ));

    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let mut stdout = std::io::stdout();
    let mut buffer: Vec<u8> = Vec::new();
    loop {
        buffer.clear();
        match reader.read_until(b'\n', &mut buffer) {
            Ok(0) => break, // EOF: the client is gone
            Ok(_) => {
                if buffer.last() != Some(&b'\n') {
                    // EOF with a partial line: like the Swift server, never process it —
                    // the next read reports EOF and the loop ends.
                    continue;
                }
                buffer.pop();
                // Swift silently skips lines that do not decode as UTF-8.
                let Ok(line) = std::str::from_utf8(&buffer) else {
                    continue;
                };
                for message in handle_line(&shared, line) {
                    send(&mut stdout, &message);
                }
            }
            Err(_) => break,
        }
    }
    clean_up_session(&shared, "stdin closed");
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared_with_store(directory: &std::path::Path) -> SharedServerState {
        Arc::new(Mutex::new(ServerState {
            store: StateStore::new(directory.to_path_buf()),
            session_id: "claude-chat-test".to_string(),
            session_label: "Claude Chat".to_string(),
            last_written_state: None,
            last_write_epoch_secs: now_epoch_secs(),
        }))
    }

    fn single(mut responses: Vec<Value>) -> Value {
        assert_eq!(responses.len(), 1, "expected exactly one response, got {responses:?}");
        responses.remove(0)
    }

    #[test]
    fn initialize_echoes_supported_version_and_writes_hello() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_with_store(dir.path());
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","clientInfo":{"name":"claude-ai","version":"1.0"}}}"#;
        let reply = single(handle_line(&shared, line));

        assert_eq!(reply["jsonrpc"], "2.0");
        assert_eq!(reply["id"], 1);
        assert_eq!(reply["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(reply["result"]["capabilities"], json!({"tools": {}}));
        assert_eq!(
            reply["result"]["serverInfo"],
            json!({"name": "claude-airou", "version": crate::cli::VERSION})
        );
        let instructions = reply["result"]["instructions"].as_str().unwrap();
        assert!(instructions.starts_with("This server controls the user's claude-airou desktop pet"));
        assert!(instructions.contains("Speech-bubble messages should stay under 60 characters."));
        assert!(instructions.ends_with("custom pets when the user asks for one."));

        // The hello snapshot landed in the store with the refined label as cwd.
        let snapshot = lock(&shared).store.read("claude-chat-test").unwrap();
        assert_eq!(snapshot.session_id, "claude-chat-test");
        assert_eq!(snapshot.cwd, "Claude Chat");
        assert_eq!(snapshot.state, PetState::Hello);
        assert_eq!(snapshot.message, "Hi! Ready when you are");
        assert_eq!(snapshot.last_event_name, "mcp:initialize");
        assert_eq!(snapshot.tool_name, None);
        assert_eq!(lock(&shared).last_written_state, Some(PetState::Hello));
    }

    #[test]
    fn initialize_unknown_version_answers_latest() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_with_store(dir.path());
        let line = r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#;
        let reply = single(handle_line(&shared, line));
        assert_eq!(reply["result"]["protocolVersion"], "2025-06-18");
        // Missing protocolVersion too.
        let line = r#"{"jsonrpc":"2.0","id":3,"method":"initialize","params":{}}"#;
        let reply = single(handle_line(&shared, line));
        assert_eq!(reply["result"]["protocolVersion"], "2025-06-18");
        // Every supported version is echoed back.
        for version in SUPPORTED_PROTOCOL_VERSIONS {
            let line = format!(
                r#"{{"jsonrpc":"2.0","id":4,"method":"initialize","params":{{"protocolVersion":"{version}"}}}}"#
            );
            let reply = single(handle_line(&shared, &line));
            assert_eq!(reply["result"]["protocolVersion"], version);
        }
    }

    #[test]
    fn initialize_without_id_still_writes_hello_but_stays_silent() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_with_store(dir.path());
        let line = r#"{"jsonrpc":"2.0","method":"initialize","params":{"clientInfo":{"name":"My MCP Client"}}}"#;
        assert!(handle_line(&shared, line).is_empty());
        let snapshot = lock(&shared).store.read("claude-chat-test").unwrap();
        assert_eq!(snapshot.cwd, "My MCP Client");
        assert_eq!(snapshot.state, PetState::Hello);
    }

    #[test]
    fn session_label_mapping_matches_swift() {
        assert_eq!(session_label_for_client_name(""), "Claude Chat");
        assert_eq!(session_label_for_client_name("claude-ai"), "Claude Chat");
        assert_eq!(session_label_for_client_name("Claude Desktop"), "Claude Chat");
        assert_eq!(session_label_for_client_name("claude-code"), "Claude Code");
        assert_eq!(session_label_for_client_name("Claude Code"), "Claude Code");
        assert_eq!(session_label_for_client_name("CLAUDE CODE"), "Claude Code");
        // Contains "code" but not "claude": kept verbatim.
        assert_eq!(session_label_for_client_name("Visual Studio Code"), "Visual Studio Code");
        assert_eq!(session_label_for_client_name("My MCP Client"), "My MCP Client");
        // Whitespace-only is not empty: kept verbatim, like Swift.
        assert_eq!(session_label_for_client_name("  "), "  ");
    }

    #[test]
    fn tools_list_names_all_four_tools_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_with_store(dir.path());
        let reply = single(handle_line(&shared, r#"{"jsonrpc":"2.0","id":5,"method":"tools/list"}"#));
        let tools = reply["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|tool| tool["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["pet_status", "list_pets", "preview_pet", "hatch_pet"]);
    }

    #[test]
    fn pet_status_call_writes_snapshot_and_answers_swift_text() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_with_store(dir.path());
        // Refine the label first so the snapshot carries it.
        handle_line(
            &shared,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"claude-ai"}}}"#,
        );
        let line = r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"pet_status","arguments":{"state":"working","message":"  Compiling the crate  "}}}"#;
        let reply = single(handle_line(&shared, line));
        assert_eq!(reply["result"]["isError"], false);
        assert_eq!(
            reply["result"]["content"][0]["text"],
            "The pet now shows \"working\" — “Compiling the crate”. Update it again at the next real transition."
        );

        let snapshot = lock(&shared).store.read("claude-chat-test").unwrap();
        assert_eq!(snapshot.state, PetState::Working);
        assert_eq!(snapshot.message, "Compiling the crate");
        assert_eq!(snapshot.last_event_name, "mcp:pet_status");
        assert_eq!(snapshot.cwd, "Claude Chat");
        // pet_status must NOT be followed by the server's own "thinking" write.
        assert_eq!(lock(&shared).last_written_state, Some(PetState::Working));
    }

    #[test]
    fn pet_status_defaults_and_idle_message_shape() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_with_store(dir.path());
        let reply = single(handle_line(
            &shared,
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"pet_status","arguments":{"state":"done"}}}"#,
        ));
        assert_eq!(
            reply["result"]["content"][0]["text"],
            "The pet now shows \"done\" — “Done!”. Update it again at the next real transition."
        );
        // idle defaults to an empty message: no em-dash clause at all.
        let reply = single(handle_line(
            &shared,
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"pet_status","arguments":{"state":"idle"}}}"#,
        ));
        assert_eq!(
            reply["result"]["content"][0]["text"],
            "The pet now shows \"idle\". Update it again at the next real transition."
        );
        let snapshot = lock(&shared).store.read("claude-chat-test").unwrap();
        assert_eq!(snapshot.state, PetState::Idle);
        assert_eq!(snapshot.message, "");
    }

    #[test]
    fn pet_status_invalid_state_is_tool_error_not_rpc_error() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_with_store(dir.path());
        let reply = single(handle_line(
            &shared,
            r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"pet_status","arguments":{"state":"nope"}}}"#,
        ));
        assert!(reply.get("error").is_none());
        assert_eq!(reply["result"]["isError"], true);
        assert_eq!(
            reply["result"]["content"][0]["text"],
            "`state` must be one of: thinking, working, needs_input, done, error, idle, hello"
        );
        // Nothing was written — not even the post-tool "thinking".
        assert!(lock(&shared).store.read("claude-chat-test").is_none());
    }

    #[test]
    fn successful_non_pet_status_tool_writes_thinking() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_with_store(dir.path());
        let reply = single(handle_line(
            &shared,
            r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"list_pets"}}"#,
        ));
        assert_eq!(reply["result"]["isError"], false);
        let text = reply["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("airou-felyne — "));
        assert!(text.contains(" ← selected"));
        assert!(text.ends_with(
            "The user switches pets via the menu bar 🐾 → Pet (use \"Reload pets\" after hatching while the overlay is running)."
        ));

        let snapshot = lock(&shared).store.read("claude-chat-test").unwrap();
        assert_eq!(snapshot.state, PetState::Thinking);
        assert_eq!(snapshot.message, "Thinking…");
        assert_eq!(snapshot.last_event_name, "mcp:tools/call:list_pets");
    }

    #[test]
    fn failed_tool_call_does_not_write_thinking() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_with_store(dir.path());
        // preview_pet without an id fails before touching the renderer.
        let reply = single(handle_line(
            &shared,
            r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"preview_pet","arguments":{}}}"#,
        ));
        assert_eq!(reply["result"]["isError"], true);
        assert_eq!(reply["result"]["content"][0]["text"], "`id` is required (see list_pets)");
        assert!(lock(&shared).store.read("claude-chat-test").is_none());
    }

    #[test]
    fn unknown_tool_answers_32602() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_with_store(dir.path());
        let reply = single(handle_line(
            &shared,
            r#"{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"bogus"}}"#,
        ));
        assert_eq!(reply["error"]["code"], -32602);
        assert_eq!(reply["error"]["message"], "Unknown tool: bogus");
        // Without an id there is no reply at all.
        assert!(handle_line(
            &shared,
            r#"{"jsonrpc":"2.0","method":"tools/call","params":{"name":"bogus"}}"#
        )
        .is_empty());
    }

    #[test]
    fn unknown_method_answers_32601_only_with_id() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_with_store(dir.path());
        let reply = single(handle_line(
            &shared,
            r#"{"jsonrpc":"2.0","id":13,"method":"resources/list"}"#,
        ));
        assert_eq!(reply["error"]["code"], -32601);
        assert_eq!(reply["error"]["message"], "Method not found: resources/list");
        assert!(handle_line(&shared, r#"{"jsonrpc":"2.0","method":"resources/list"}"#).is_empty());
    }

    #[test]
    fn notifications_and_responses_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_with_store(dir.path());
        assert!(handle_line(&shared, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_empty());
        assert!(handle_line(&shared, r#"{"jsonrpc":"2.0","method":"notifications/cancelled","id":1}"#).is_empty());
        // A response to a server-initiated request (no method): nothing to match.
        assert!(handle_line(&shared, r#"{"jsonrpc":"2.0","id":1,"result":{}}"#).is_empty());
        // A non-string method is treated the same way.
        assert!(handle_line(&shared, r#"{"jsonrpc":"2.0","id":1,"method":42}"#).is_empty());
    }

    #[test]
    fn ping_answers_empty_result() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_with_store(dir.path());
        let reply = single(handle_line(&shared, r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#));
        assert_eq!(reply, json!({"jsonrpc": "2.0", "id": 7, "result": {}}));
        assert!(handle_line(&shared, r#"{"jsonrpc":"2.0","method":"ping"}"#).is_empty());
    }

    #[test]
    fn parse_error_and_invalid_request_shapes() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_with_store(dir.path());
        let reply = single(handle_line(&shared, "this is not json {"));
        assert_eq!(
            reply,
            json!({"jsonrpc": "2.0", "id": Value::Null, "error": {"code": -32700, "message": "Parse error"}})
        );
        // Top-level scalar: JSONSerialization would refuse it too → parse error.
        let reply = single(handle_line(&shared, "42"));
        assert_eq!(reply["error"]["code"], -32700);
        // Array with non-object elements → invalid request.
        let reply = single(handle_line(&shared, "[1,2]"));
        assert_eq!(
            reply,
            json!({"jsonrpc": "2.0", "id": Value::Null, "error": {"code": -32600, "message": "Invalid request"}})
        );
        // Blank lines produce nothing.
        assert!(handle_line(&shared, "").is_empty());
        assert!(handle_line(&shared, "   \r").is_empty());
    }

    #[test]
    fn batch_arrays_are_handled_per_element() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_with_store(dir.path());
        let replies = handle_line(
            &shared,
            r#"[{"jsonrpc":"2.0","id":1,"method":"ping"},{"jsonrpc":"2.0","method":"notifications/x"},{"jsonrpc":"2.0","id":2,"method":"ping"}]"#,
        );
        assert_eq!(replies.len(), 2);
        assert_eq!(replies[0]["id"], 1);
        assert_eq!(replies[1]["id"], 2);
        // An empty batch is fine and silent.
        assert!(handle_line(&shared, "[]").is_empty());
    }

    #[test]
    fn watchdog_resets_stale_busy_state_only() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_with_store(dir.path());

        // Nothing written yet: tick is a no-op.
        idle_watchdog_tick(&shared);
        assert!(lock(&shared).store.read("claude-chat-test").is_none());

        // Fresh busy state: left alone.
        write_state(&shared, PetState::Working, "Working on it…", "mcp:pet_status");
        idle_watchdog_tick(&shared);
        assert_eq!(
            lock(&shared).store.read("claude-chat-test").unwrap().state,
            PetState::Working
        );

        // Stale busy state: reset to idle via the watchdog event.
        lock(&shared).last_write_epoch_secs = now_epoch_secs() - BUSY_IDLE_AFTER_SECS - 1.0;
        idle_watchdog_tick(&shared);
        let snapshot = lock(&shared).store.read("claude-chat-test").unwrap();
        assert_eq!(snapshot.state, PetState::Idle);
        assert_eq!(snapshot.message, "");
        assert_eq!(snapshot.last_event_name, "mcp:idle-watchdog");

        // Stale attention state: keeps the longer decay, watchdog does not touch it.
        write_state(&shared, PetState::NeedsInput, "Your turn!", "mcp:pet_status");
        lock(&shared).last_write_epoch_secs = now_epoch_secs() - BUSY_IDLE_AFTER_SECS - 1.0;
        idle_watchdog_tick(&shared);
        assert_eq!(
            lock(&shared).store.read("claude-chat-test").unwrap().state,
            PetState::NeedsInput
        );
    }

    #[test]
    fn clean_up_removes_the_session_file() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_with_store(dir.path());
        write_state(&shared, PetState::Hello, "Hi! Ready when you are", "mcp:initialize");
        assert!(lock(&shared).store.read("claude-chat-test").is_some());
        clean_up_session(&shared, "stdin closed");
        assert!(lock(&shared).store.read("claude-chat-test").is_none());
    }

    #[test]
    fn responses_serialize_to_single_lines() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_with_store(dir.path());
        let reply = single(handle_line(
            &shared,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
        ));
        let mut out: Vec<u8> = Vec::new();
        send(&mut out, &reply);
        assert_eq!(out.last(), Some(&b'\n'));
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.matches('\n').count(), 1, "protocol messages must be single lines");
    }
}
