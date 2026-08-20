//! Pure mapping from a Claude Code hook event to what the pet should do, plus the merge
//! policy for concurrent events. Port of `Hook/HookEventMapper.swift` (incl. ToolSummarizer)
//! and `Hook/HookMergePolicy.swift` — behaviour must match 1:1.

use crate::model::{ActiveAgent, PetState, SessionSnapshot};
use serde_json::Value;

/// Events the installer registers. Anything else is ignored if it ever arrives.
// Consumed by `install::run_install_hooks` (still a stub while modules land in parallel);
// allow(dead_code) keeps the transitional build warning-free without changing the signature.
#[allow(dead_code)]
pub const SUBSCRIBED_EVENT_NAMES: [&str; 17] = [
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PostToolBatch",
    "PostToolUseFailure",
    "PermissionRequest",
    "Notification",
    "Stop",
    "StopFailure",
    "SubagentStart",
    "SubagentStop",
    "PreCompact",
    "PostCompact",
    "Elicitation",
    "ElicitationResult",
];

/// The subagents working for a session right now, after applying this event.
///
/// Any event carrying an `agent_id` refreshes that agent — relying on `SubagentStart`
/// alone would miss agents that were already running when the hook was installed.
/// `SubagentStop` retires one, and a turn boundary retires all.
///
/// Every entry carries the last time we heard from it, because none of those signals can
/// be trusted on its own: a stop can be dropped by the merge policy (the session was
/// showing a result, or waiting on the user), lost to a race between two hook processes
/// writing the same file, or never sent. Anything unheard-of for
/// `AGENT_ALIVE_WINDOW_SECS` stops counting, so those leaks expire instead of haunting
/// the pet until the next prompt.
fn agents_after(
    existing: Option<&SessionSnapshot>,
    input: &HookInput,
    now_secs: f64,
) -> Vec<ActiveAgent> {
    const TURN_BOUNDARY_EVENT_NAMES: [&str; 3] = ["UserPromptSubmit", "Stop", "StopFailure"];
    let is_turn_boundary = TURN_BOUNDARY_EVENT_NAMES.contains(&input.hook_event_name())
        // SessionStart also fires mid-turn after compaction, with the agents still running.
        || (input.hook_event_name() == "SessionStart" && input.session_start_source() != Some("compact"));
    if is_turn_boundary {
        return Vec::new();
    }

    let mut agents: Vec<ActiveAgent> = existing
        .map(|snapshot| snapshot.active_agents.clone())
        .unwrap_or_default()
        .into_iter()
        .filter(|agent| now_secs - agent.last_seen_epoch_seconds <= SessionSnapshot::AGENT_ALIVE_WINDOW_SECS)
        .collect();

    let Some(agent_id) = input.agent_id() else {
        return agents;
    };
    if input.hook_event_name() == "SubagentStop" {
        agents.retain(|agent| agent.id != agent_id);
        return agents;
    }
    match agents.iter_mut().find(|agent| agent.id == agent_id) {
        Some(agent) => agent.last_seen_epoch_seconds = now_secs,
        None => agents.push(ActiveAgent {
            id: agent_id.to_string(),
            last_seen_epoch_seconds: now_secs,
        }),
    }
    if agents.len() > SessionSnapshot::MAXIMUM_TRACKED_AGENTS {
        // Drop the stalest, not the oldest-added: the long-running agent is the one still
        // working, and a burst of short-lived ones is what pushes the list over.
        agents.sort_by(|a, b| {
            a.last_seen_epoch_seconds
                .partial_cmp(&b.last_seen_epoch_seconds)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let excess = agents.len() - SessionSnapshot::MAXIMUM_TRACKED_AGENTS;
        agents.drain(0..excess);
    }
    agents
}

/// Subagent events that mean "an agent finished a piece of work". They routinely trail
/// the main thread's own Stop, so a session already showing a result ignores them; the
/// starting half (`SubagentStart`, an agent's `PreToolUse`) is real activity and is not
/// listed here.
const SUBAGENT_WIND_DOWN_EVENT_NAMES: [&str; 4] =
    ["SubagentStop", "PostToolUse", "PostToolUseFailure", "PostToolBatch"];

/// The subset of the hook stdin JSON the pet cares about (accessors over the raw object,
/// like Swift's `HookInput`).
pub struct HookInput {
    pub raw: Value,
}

impl HookInput {
    pub fn parse(data: &[u8]) -> Option<HookInput> {
        let raw: Value = serde_json::from_slice(data).ok()?;
        raw.is_object().then_some(HookInput { raw })
    }

    fn string(&self, key: &str) -> Option<&str> {
        self.raw.get(key).and_then(Value::as_str)
    }

    pub fn session_id(&self) -> &str {
        self.string("session_id").unwrap_or("unknown-session")
    }
    pub fn cwd(&self) -> String {
        self.string("cwd").map(str::to_string).unwrap_or_else(|| {
            std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default()
        })
    }
    pub fn hook_event_name(&self) -> &str {
        self.string("hook_event_name").unwrap_or("")
    }
    pub fn transcript_path(&self) -> Option<&str> {
        self.string("transcript_path")
    }
    pub fn tool_name(&self) -> Option<&str> {
        self.string("tool_name")
    }
    pub fn tool_input(&self) -> &Value {
        self.raw.get("tool_input").unwrap_or(&Value::Null)
    }
    pub fn tool_use_id(&self) -> Option<&str> {
        self.string("tool_use_id")
    }
    pub fn notification_type(&self) -> Option<&str> {
        self.string("notification_type")
    }
    pub fn notification_message(&self) -> Option<&str> {
        self.string("message")
    }
    pub fn error_type(&self) -> Option<&str> {
        self.string("error_type")
    }
    pub fn session_start_source(&self) -> Option<&str> {
        self.string("source")
    }
    pub fn agent_type(&self) -> Option<&str> {
        self.string("agent_type")
    }
    pub fn agent_id(&self) -> Option<&str> {
        self.string("agent_id")
    }
    pub fn compact_trigger(&self) -> Option<&str> {
        self.string("trigger")
    }
    pub fn mcp_server_name(&self) -> Option<&str> {
        self.string("mcp_server_name")
    }
    pub fn elicitation_action(&self) -> Option<&str> {
        self.string("action")
    }
    pub fn is_subagent_event(&self) -> bool {
        self.agent_id().is_some()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MappingResult {
    Update {
        state: PetState,
        message: String,
        tool_name: Option<String>,
    },
    RemoveSession,
    Ignore,
}

fn update(state: PetState, message: impl Into<String>, tool_name: Option<&str>) -> MappingResult {
    MappingResult::Update {
        state,
        message: message.into(),
        tool_name: tool_name.map(str::to_string),
    }
}

/// Port of `HookEventMapper.map(_:)` — every branch, message string and state must match
/// the Swift original exactly (see the table in README.md).
pub fn map(input: &HookInput) -> MappingResult {
    match input.hook_event_name() {
        "SessionStart" => match input.session_start_source() {
            // Fires mid-turn after compaction; Claude keeps working, so don't greet.
            Some("compact") => update(PetState::Thinking, "Context compacted, back to work", None),
            Some("resume") => update(PetState::Hello, "Welcome back!", None),
            Some("clear") => update(PetState::Hello, "Fresh start!", None),
            _ => update(PetState::Hello, "Hi! Ready when you are", None),
        },

        "SessionEnd" => MappingResult::RemoveSession,

        "UserPromptSubmit" => update(PetState::Thinking, "Thinking…", None),

        "PreToolUse" => {
            if let Some(interaction) = map_user_interaction_tool(input) {
                return interaction;
            }
            let summary = summarize_tool(input.tool_name().unwrap_or("tool"), input.tool_input());
            update(PetState::Working, summary, input.tool_name())
        }

        "PostToolUse" | "PostToolBatch" => update(PetState::Thinking, "Thinking…", input.tool_name()),

        "PostToolUseFailure" => {
            let name = input.tool_name().unwrap_or("tool");
            update(PetState::Error, format!("{name} failed — recovering…"), input.tool_name())
        }

        "PermissionRequest" => {
            if let Some(interaction) = map_user_interaction_tool(input) {
                return interaction;
            }
            let summary = summarize_tool(input.tool_name().unwrap_or("tool"), input.tool_input());
            update(PetState::WaitingApproval, format!("Approve? {summary}"), input.tool_name())
        }

        "Notification" => map_notification(input),

        "Stop" => update(PetState::Done, "Done!", None),

        "StopFailure" => {
            let detail = input
                .error_type()
                .map(|t| t.replace('_', " "))
                .unwrap_or_else(|| "API error".to_string());
            update(PetState::Error, format!("Stopped: {detail}"), None)
        }

        "SubagentStart" => {
            let agent = input.agent_type().unwrap_or("sub");
            update(PetState::Working, format!("Sent a {agent} agent to work"), Some("Agent"))
        }

        "SubagentStop" => update(PetState::Thinking, "Agent reported back", Some("Agent")),

        "PreCompact" => {
            let trigger = if input.compact_trigger() == Some("auto") { "Auto-compacting" } else { "Compacting" };
            update(PetState::Thinking, format!("{trigger} context…"), None)
        }

        "PostCompact" => update(PetState::Thinking, "Context compacted", None),

        "Elicitation" => {
            // Elicitation input carries mcp_server_name + message (no tool_name).
            let text = input.notification_message().unwrap_or("").trim();
            let server = input
                .mcp_server_name()
                .map(|s| format!(" ({s})"))
                .unwrap_or_default();
            let message = if text.is_empty() {
                format!("A tool needs your input{server}")
            } else {
                truncate(text)
            };
            update(PetState::NeedsInput, message, input.mcp_server_name())
        }

        "ElicitationResult" => match input.elicitation_action() {
            Some("decline") | Some("cancel") => {
                update(PetState::Thinking, "Okay, skipping that", input.mcp_server_name())
            }
            _ => update(PetState::Working, "Thanks! Continuing…", input.mcp_server_name()),
        },

        _ => MappingResult::Ignore,
    }
}

/// Tools that block on the user are "needs input", not "working"/"approve?".
fn map_user_interaction_tool(input: &HookInput) -> Option<MappingResult> {
    match input.tool_name() {
        Some("AskUserQuestion") => Some(update(PetState::NeedsInput, "Asking you a question", input.tool_name())),
        Some("ExitPlanMode") => Some(update(PetState::NeedsInput, "Waiting for plan approval", input.tool_name())),
        _ => None,
    }
}

fn map_notification(input: &HookInput) -> MappingResult {
    let text = input.notification_message().unwrap_or("").trim();
    match input.notification_type() {
        Some("permission_prompt") => {
            let message = if text.is_empty() { "Needs your approval".to_string() } else { truncate(text) };
            update(PetState::WaitingApproval, message, input.tool_name())
        }
        // "Claude finished ~60 s ago and you haven't typed": the session is simply idle at the
        // prompt. Writing idle also clears a stuck busy state after an interrupt (Stop does not fire then).
        Some("idle_prompt") => update(PetState::Idle, "", None),
        Some("agent_needs_input") => {
            let message = if text.is_empty() { "Needs your input".to_string() } else { truncate(text) };
            update(PetState::NeedsInput, message, None)
        }
        Some("elicitation_dialog") | Some("elicitation_url_dialog") => {
            let message = if text.is_empty() { "A tool needs your input".to_string() } else { truncate(text) };
            update(PetState::NeedsInput, message, None)
        }
        Some("agent_completed") => {
            let message = if text.is_empty() { "Done!".to_string() } else { truncate(text) };
            update(PetState::Done, message, None)
        }
        Some("auth_success") | Some("elicitation_complete") | Some("elicitation_response") => MappingResult::Ignore,
        _ => MappingResult::Ignore,
    }
}

/// Two full bubble lines (~300pt wide) hold roughly this much; longer text gets an ellipsis.
const MAX_CHARACTERS: usize = 100;

/// Port of `ToolSummarizer.summarize` — one bubble line per tool call.
pub fn summarize_tool(tool_name: &str, tool_input: &Value) -> String {
    let last_path_component = |key: &str| -> Option<String> {
        let path = tool_input.get(key)?.as_str()?;
        if path.is_empty() {
            return None;
        }
        Some(
            std::path::Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string()),
        )
    };
    let value = |key: &str| -> Option<String> {
        let text = tool_input.get(key)?.as_str()?;
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    };

    let line: String = match tool_name {
        "Read" => format!("Reading {}", last_path_component("file_path").unwrap_or_else(|| "a file".into())),
        "Edit" | "MultiEdit" => {
            format!("Editing {}", last_path_component("file_path").unwrap_or_else(|| "a file".into()))
        }
        "Write" => format!("Writing {}", last_path_component("file_path").unwrap_or_else(|| "a file".into())),
        "NotebookEdit" => {
            format!("Editing {}", last_path_component("notebook_path").unwrap_or_else(|| "a notebook".into()))
        }
        "Bash" => {
            let description = value("description");
            let command = value("command").map(|c| c.replace('\n', " "));
            format!("Running: {}", description.or(command).unwrap_or_else(|| "a command".into()))
        }
        "Grep" => format!("Searching for “{}”", value("pattern").unwrap_or_else(|| "…".into())),
        "Glob" => format!("Looking for {}", value("pattern").unwrap_or_else(|| "files".into())),
        "WebFetch" => match value("url").as_deref().and_then(url_host) {
            Some(host) => format!("Fetching {host}"),
            None => "Fetching a page".to_string(),
        },
        "WebSearch" => format!("Searching the web: {}", value("query").unwrap_or_else(|| "…".into())),
        "Agent" | "Task" => format!("Delegating: {}", value("description").unwrap_or_else(|| "a subtask".into())),
        "TodoWrite" | "TaskCreate" | "TaskUpdate" => "Updating the task list".to_string(),
        "AskUserQuestion" => "Asking you a question".to_string(),
        "ExitPlanMode" => "Waiting for plan approval".to_string(),
        "Skill" => format!("Using skill {}", value("skill").unwrap_or_default()),
        _ => {
            if tool_name.starts_with("mcp__") {
                let readable = tool_name
                    .split('_')
                    .filter(|part| !part.is_empty())
                    .skip(1)
                    .collect::<Vec<_>>()
                    .join(" ");
                if readable.is_empty() {
                    "Using an MCP tool".to_string()
                } else {
                    format!("Using {readable}")
                }
            } else {
                format!("Using {tool_name}")
            }
        }
    };
    truncate(&line)
}

/// Extracts the host from a URL string without external crates: the authority part sits
/// between `://` and the next `/`, `?` or `#`; strip userinfo (before the last `@`) and the
/// port (after `:`, unless the host is a bracketed IPv6 literal). Mirrors Swift's
/// `URL(string:)?.host` for the URLs the WebFetch tool actually sees.
fn url_host(url: &str) -> Option<String> {
    let after_scheme = url.split_once("://")?.1;
    let authority_end = after_scheme
        .find(|c| c == '/' || c == '?' || c == '#')
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_end];
    // Strip userinfo — everything before the last '@'.
    let host_port = authority.rsplit_once('@').map(|(_, rest)| rest).unwrap_or(authority);
    // Bracketed IPv6 literal: host is the part inside the brackets.
    let host = if let Some(inner) = host_port.strip_prefix('[') {
        inner.split_once(']').map(|(h, _)| h).unwrap_or(inner)
    } else {
        host_port.split_once(':').map(|(h, _)| h).unwrap_or(host_port)
    };
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

/// Port of `ToolSummarizer.truncate` (100 chars max, ellipsis).
///
/// Known deviation: Swift `String.count` counts extended grapheme clusters; `chars()`
/// counts Unicode scalars, so ZWJ emoji and NFD-decomposed text hit the limit earlier here
/// (never a panic — `char` boundaries are always valid). Exact parity would need a
/// grapheme-segmentation dependency; the bubble is a preview, so the trade is accepted.
pub fn truncate(text: &str) -> String {
    if text.chars().count() <= MAX_CHARACTERS {
        return text.to_string();
    }
    let mut truncated: String = text.chars().take(MAX_CHARACTERS - 1).collect();
    truncated.push('…');
    truncated
}

#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    /// The event itself is chatter, but it changed who is working for the session: write
    /// the roster and leave state, message and timing exactly as they were. Without this
    /// every `Keep` below would silently drop a `SubagentStop` — and a stop is sent once.
    RosterOnly(SessionSnapshot),
    Write(SessionSnapshot),
    Keep(String),
}

/// Port of `HookMergePolicy.resolve` — decides whether a freshly mapped event may overwrite
/// the session's current snapshot (protects waiting_approval/needs_input from sibling tools
/// and subagent chatter; carries pending_tool_use_id).
pub fn resolve(
    existing: Option<&SessionSnapshot>,
    input: &HookInput,
    mapped_state: PetState,
    message: &str,
    tool_name: Option<&str>,
    now_secs: f64,
) -> Resolution {
    let existing_state = existing.map(SessionSnapshot::effective_state).unwrap_or(PetState::Idle);
    let user_is_blocked = existing_state.is_attention_needed();

    // Who is working for this session, whatever we decide about the event itself.
    let agents = agents_after(existing, input, now_secs);
    let keep = |reason: String| match existing {
        Some(existing) if existing.active_agents != agents => {
            let mut snapshot = existing.clone();
            snapshot.active_agents = agents.clone();
            Resolution::RosterOnly(snapshot)
        }
        _ => Resolution::Keep(reason),
    };

    // "Claude finished ~60 s ago and you haven't typed" — the finished/failed result is
    // exactly what the user still wants to see, so idle_prompt only clears busy states.
    if input.notification_type() == Some("idle_prompt")
        && matches!(existing_state, PetState::Done | PetState::Error)
    {
        return keep(format!("idle_prompt while {}", existing_state.raw()));
    }

    // A subagent's own Stop routinely lands a second or two after the main thread's Stop.
    // Taking it at face value un-finishes a session that is done: the pet drops the check
    // mark back to thinking and sits there until the busy state decays, while Claude Code
    // rightly shows the session as finished.
    //
    // Only the winding-down half of subagent traffic is chatter, though. An agent that
    // *starts* something after the turn ended is real work — background workflows run
    // exactly like that — and ignoring it left the pet showing done while a dozen agents
    // were grinding away.
    let result_is_final = matches!(existing_state, PetState::Done | PetState::Error);
    let is_subagent_winding_down = input.is_subagent_event()
        && SUBAGENT_WIND_DOWN_EVENT_NAMES.contains(&input.hook_event_name());

    if result_is_final && is_subagent_winding_down {
        return keep(format!(
            "subagent {} while {}",
            input.hook_event_name(),
            existing_state.raw()
        ));
    }

    if user_is_blocked {
        if let Some(existing) = existing {
            // Subagents keep running while the main thread waits on the user; ignore their chatter.
            if input.is_subagent_event() {
                return keep(format!(
                    "subagent {} while {}",
                    input.hook_event_name(),
                    existing.state.raw()
                ));
            }
            // A sibling tool from the same batch finished — the awaited call is still pending.
            let is_tool_completion = input.hook_event_name() == "PostToolUse"
                || input.hook_event_name() == "PostToolUseFailure";
            if is_tool_completion {
                if let (Some(pending), Some(finished)) =
                    (existing.pending_tool_use_id.as_deref(), input.tool_use_id())
                {
                    if pending != finished {
                        return keep(format!(
                            "sibling tool {finished} finished while waiting on {pending}"
                        ));
                    }
                }
            }
        }
    }

    let mut snapshot = SessionSnapshot {
        session_id: input.session_id().to_string(),
        cwd: input.cwd(),
        state: mapped_state,
        message: message.to_string(),
        last_event_name: input.hook_event_name().to_string(),
        tool_name: tool_name.map(str::to_string),
        updated_at_epoch_seconds: now_secs,
        pending_tool_use_id: None,
        active_agents: agents,
    };
    if mapped_state.is_attention_needed() {
        // PermissionRequest / PreToolUse(AskUserQuestion) carry tool_use_id; a Notification
        // re-asserting the same wait does not, so inherit the id we already had.
        snapshot.pending_tool_use_id = input
            .tool_use_id()
            .map(str::to_string)
            .or_else(|| {
                if user_is_blocked {
                    existing.and_then(|e| e.pending_tool_use_id.clone())
                } else {
                    None
                }
            });
    }
    Resolution::Write(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::now_epoch_secs;
    use serde_json::json;

    fn input(raw: Value) -> HookInput {
        HookInput { raw }
    }

    fn assert_update(result: MappingResult, state: PetState, message: &str, tool_name: Option<&str>) {
        assert_eq!(
            result,
            MappingResult::Update {
                state,
                message: message.to_string(),
                tool_name: tool_name.map(str::to_string),
            }
        );
    }

    // MARK: mapper events

    #[test]
    fn session_start_sources() {
        let default = input(json!({"hook_event_name": "SessionStart"}));
        assert_update(map(&default), PetState::Hello, "Hi! Ready when you are", None);
        let startup = input(json!({"hook_event_name": "SessionStart", "source": "startup"}));
        assert_update(map(&startup), PetState::Hello, "Hi! Ready when you are", None);
        let compact = input(json!({"hook_event_name": "SessionStart", "source": "compact"}));
        assert_update(map(&compact), PetState::Thinking, "Context compacted, back to work", None);
        let resume = input(json!({"hook_event_name": "SessionStart", "source": "resume"}));
        assert_update(map(&resume), PetState::Hello, "Welcome back!", None);
        let clear = input(json!({"hook_event_name": "SessionStart", "source": "clear"}));
        assert_update(map(&clear), PetState::Hello, "Fresh start!", None);
    }

    #[test]
    fn session_end_removes() {
        let end = input(json!({"hook_event_name": "SessionEnd", "reason": "exit"}));
        assert_eq!(map(&end), MappingResult::RemoveSession);
    }

    #[test]
    fn user_prompt_submit_thinks() {
        let event = input(json!({"hook_event_name": "UserPromptSubmit"}));
        assert_update(map(&event), PetState::Thinking, "Thinking…", None);
    }

    #[test]
    fn pre_tool_use_summarizes() {
        let event = input(json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "Read",
            "tool_input": {"file_path": "/home/me/project/main.rs"}
        }));
        assert_update(map(&event), PetState::Working, "Reading main.rs", Some("Read"));
    }

    #[test]
    fn pre_tool_use_without_tool_name() {
        let event = input(json!({"hook_event_name": "PreToolUse"}));
        assert_update(map(&event), PetState::Working, "Using tool", None);
    }

    #[test]
    fn user_interaction_tools_map_to_needs_input() {
        for event_name in ["PreToolUse", "PermissionRequest"] {
            let ask = input(json!({"hook_event_name": event_name, "tool_name": "AskUserQuestion"}));
            assert_update(map(&ask), PetState::NeedsInput, "Asking you a question", Some("AskUserQuestion"));
            let plan = input(json!({"hook_event_name": event_name, "tool_name": "ExitPlanMode"}));
            assert_update(map(&plan), PetState::NeedsInput, "Waiting for plan approval", Some("ExitPlanMode"));
        }
    }

    #[test]
    fn post_tool_use_and_batch_think() {
        let post = input(json!({"hook_event_name": "PostToolUse", "tool_name": "Bash"}));
        assert_update(map(&post), PetState::Thinking, "Thinking…", Some("Bash"));
        let batch = input(json!({"hook_event_name": "PostToolBatch"}));
        assert_update(map(&batch), PetState::Thinking, "Thinking…", None);
    }

    #[test]
    fn post_tool_use_failure() {
        let failure = input(json!({"hook_event_name": "PostToolUseFailure", "tool_name": "Bash"}));
        assert_update(map(&failure), PetState::Error, "Bash failed — recovering…", Some("Bash"));
        let anonymous = input(json!({"hook_event_name": "PostToolUseFailure"}));
        assert_update(map(&anonymous), PetState::Error, "tool failed — recovering…", None);
    }

    #[test]
    fn permission_request_asks_for_approval() {
        let event = input(json!({
            "hook_event_name": "PermissionRequest",
            "tool_name": "Bash",
            "tool_input": {"command": "rm -rf build"}
        }));
        assert_update(
            map(&event),
            PetState::WaitingApproval,
            "Approve? Running: rm -rf build",
            Some("Bash"),
        );
    }

    #[test]
    fn notification_permission_prompt() {
        let with_text = input(json!({
            "hook_event_name": "Notification",
            "notification_type": "permission_prompt",
            "message": "  Claude needs permission to use Bash  ",
            "tool_name": "Bash"
        }));
        assert_update(
            map(&with_text),
            PetState::WaitingApproval,
            "Claude needs permission to use Bash",
            Some("Bash"),
        );
        let empty = input(json!({
            "hook_event_name": "Notification",
            "notification_type": "permission_prompt",
            "message": "   "
        }));
        assert_update(map(&empty), PetState::WaitingApproval, "Needs your approval", None);
    }

    #[test]
    fn notification_idle_prompt_clears_to_idle() {
        let event = input(json!({
            "hook_event_name": "Notification",
            "notification_type": "idle_prompt",
            "message": "Claude is waiting for your input"
        }));
        assert_update(map(&event), PetState::Idle, "", None);
    }

    #[test]
    fn notification_agent_needs_input() {
        let event = input(json!({
            "hook_event_name": "Notification",
            "notification_type": "agent_needs_input",
            "message": ""
        }));
        assert_update(map(&event), PetState::NeedsInput, "Needs your input", None);
        let with_text = input(json!({
            "hook_event_name": "Notification",
            "notification_type": "agent_needs_input",
            "message": "Pick one"
        }));
        assert_update(map(&with_text), PetState::NeedsInput, "Pick one", None);
    }

    #[test]
    fn notification_elicitation_dialogs() {
        for kind in ["elicitation_dialog", "elicitation_url_dialog"] {
            let event = input(json!({
                "hook_event_name": "Notification",
                "notification_type": kind
            }));
            assert_update(map(&event), PetState::NeedsInput, "A tool needs your input", None);
        }
    }

    #[test]
    fn notification_agent_completed() {
        let event = input(json!({
            "hook_event_name": "Notification",
            "notification_type": "agent_completed"
        }));
        assert_update(map(&event), PetState::Done, "Done!", None);
        let with_text = input(json!({
            "hook_event_name": "Notification",
            "notification_type": "agent_completed",
            "message": "All tasks finished"
        }));
        assert_update(map(&with_text), PetState::Done, "All tasks finished", None);
    }

    #[test]
    fn notification_ignored_types() {
        for kind in ["auth_success", "elicitation_complete", "elicitation_response", "something_new"] {
            let event = input(json!({
                "hook_event_name": "Notification",
                "notification_type": kind,
                "message": "text"
            }));
            assert_eq!(map(&event), MappingResult::Ignore, "type {kind} should be ignored");
        }
        let untyped = input(json!({"hook_event_name": "Notification", "message": "text"}));
        assert_eq!(map(&untyped), MappingResult::Ignore);
    }

    #[test]
    fn stop_is_done() {
        let event = input(json!({"hook_event_name": "Stop"}));
        assert_update(map(&event), PetState::Done, "Done!", None);
    }

    #[test]
    fn stop_failure_replaces_underscores() {
        let typed = input(json!({"hook_event_name": "StopFailure", "error_type": "rate_limit_exceeded"}));
        assert_update(map(&typed), PetState::Error, "Stopped: rate limit exceeded", None);
        let untyped = input(json!({"hook_event_name": "StopFailure"}));
        assert_update(map(&untyped), PetState::Error, "Stopped: API error", None);
    }

    #[test]
    fn subagent_start_and_stop() {
        let start = input(json!({"hook_event_name": "SubagentStart", "agent_type": "explore"}));
        assert_update(map(&start), PetState::Working, "Sent a explore agent to work", Some("Agent"));
        let anonymous = input(json!({"hook_event_name": "SubagentStart"}));
        assert_update(map(&anonymous), PetState::Working, "Sent a sub agent to work", Some("Agent"));
        let stop = input(json!({"hook_event_name": "SubagentStop"}));
        assert_update(map(&stop), PetState::Thinking, "Agent reported back", Some("Agent"));
    }

    #[test]
    fn compaction_events() {
        let auto = input(json!({"hook_event_name": "PreCompact", "trigger": "auto"}));
        assert_update(map(&auto), PetState::Thinking, "Auto-compacting context…", None);
        let manual = input(json!({"hook_event_name": "PreCompact", "trigger": "manual"}));
        assert_update(map(&manual), PetState::Thinking, "Compacting context…", None);
        let untriggered = input(json!({"hook_event_name": "PreCompact"}));
        assert_update(map(&untriggered), PetState::Thinking, "Compacting context…", None);
        let post = input(json!({"hook_event_name": "PostCompact"}));
        assert_update(map(&post), PetState::Thinking, "Context compacted", None);
    }

    #[test]
    fn elicitation_with_and_without_message() {
        let with_text = input(json!({
            "hook_event_name": "Elicitation",
            "mcp_server_name": "github",
            "message": "  Enter your token  "
        }));
        assert_update(map(&with_text), PetState::NeedsInput, "Enter your token", Some("github"));
        let empty_with_server = input(json!({
            "hook_event_name": "Elicitation",
            "mcp_server_name": "github",
            "message": "   "
        }));
        assert_update(
            map(&empty_with_server),
            PetState::NeedsInput,
            "A tool needs your input (github)",
            Some("github"),
        );
        let bare = input(json!({"hook_event_name": "Elicitation"}));
        assert_update(map(&bare), PetState::NeedsInput, "A tool needs your input", None);
    }

    #[test]
    fn elicitation_result_actions() {
        for action in ["decline", "cancel"] {
            let event = input(json!({
                "hook_event_name": "ElicitationResult",
                "action": action,
                "mcp_server_name": "github"
            }));
            assert_update(map(&event), PetState::Thinking, "Okay, skipping that", Some("github"));
        }
        let accept = input(json!({"hook_event_name": "ElicitationResult", "action": "accept"}));
        assert_update(map(&accept), PetState::Working, "Thanks! Continuing…", None);
        let missing = input(json!({"hook_event_name": "ElicitationResult"}));
        assert_update(map(&missing), PetState::Working, "Thanks! Continuing…", None);
    }

    #[test]
    fn unknown_events_are_ignored() {
        let unknown = input(json!({"hook_event_name": "SomethingNew"}));
        assert_eq!(map(&unknown), MappingResult::Ignore);
        let empty = input(json!({}));
        assert_eq!(map(&empty), MappingResult::Ignore);
    }

    // MARK: summarizer

    #[test]
    fn summarize_file_tools() {
        assert_eq!(summarize_tool("Read", &json!({"file_path": "/a/b/main.swift"})), "Reading main.swift");
        assert_eq!(summarize_tool("Read", &json!({})), "Reading a file");
        assert_eq!(summarize_tool("Read", &json!({"file_path": ""})), "Reading a file");
        assert_eq!(summarize_tool("Edit", &json!({"file_path": "/x/lib.rs"})), "Editing lib.rs");
        assert_eq!(summarize_tool("MultiEdit", &json!({"file_path": "/x/lib.rs"})), "Editing lib.rs");
        assert_eq!(summarize_tool("Write", &json!({"file_path": "notes.md"})), "Writing notes.md");
        assert_eq!(summarize_tool("Write", &json!({})), "Writing a file");
        assert_eq!(
            summarize_tool("NotebookEdit", &json!({"notebook_path": "/n/analysis.ipynb"})),
            "Editing analysis.ipynb"
        );
        assert_eq!(summarize_tool("NotebookEdit", &json!({})), "Editing a notebook");
    }

    #[test]
    fn summarize_bash_prefers_description_and_collapses_newlines() {
        assert_eq!(
            summarize_tool("Bash", &json!({"description": "Build the app", "command": "make"})),
            "Running: Build the app"
        );
        assert_eq!(
            summarize_tool("Bash", &json!({"command": "cargo build\ncargo test"})),
            "Running: cargo build cargo test"
        );
        // Blank description falls through to the command.
        assert_eq!(
            summarize_tool("Bash", &json!({"description": "   ", "command": "ls"})),
            "Running: ls"
        );
        assert_eq!(summarize_tool("Bash", &json!({})), "Running: a command");
    }

    #[test]
    fn summarize_search_tools() {
        assert_eq!(summarize_tool("Grep", &json!({"pattern": "todo!"})), "Searching for “todo!”");
        assert_eq!(summarize_tool("Grep", &json!({})), "Searching for “…”");
        assert_eq!(summarize_tool("Glob", &json!({"pattern": "**/*.rs"})), "Looking for **/*.rs");
        assert_eq!(summarize_tool("Glob", &json!({})), "Looking for files");
        assert_eq!(
            summarize_tool("WebSearch", &json!({"query": "rust url parsing"})),
            "Searching the web: rust url parsing"
        );
        assert_eq!(summarize_tool("WebSearch", &json!({})), "Searching the web: …");
    }

    #[test]
    fn summarize_webfetch_extracts_host() {
        assert_eq!(
            summarize_tool("WebFetch", &json!({"url": "https://docs.rs/serde/latest"})),
            "Fetching docs.rs"
        );
        assert_eq!(
            summarize_tool("WebFetch", &json!({"url": "https://example.com:8443/path?q=1"})),
            "Fetching example.com"
        );
        assert_eq!(
            summarize_tool("WebFetch", &json!({"url": "https://user:pass@example.com/x"})),
            "Fetching example.com"
        );
        assert_eq!(
            summarize_tool("WebFetch", &json!({"url": "http://[::1]:8080/health"})),
            "Fetching ::1"
        );
        // No scheme / no host → generic line.
        assert_eq!(summarize_tool("WebFetch", &json!({"url": "example.com/foo"})), "Fetching a page");
        assert_eq!(summarize_tool("WebFetch", &json!({"url": "https:///nohost"})), "Fetching a page");
        assert_eq!(summarize_tool("WebFetch", &json!({})), "Fetching a page");
    }

    #[test]
    fn summarize_agent_and_task_tools() {
        assert_eq!(
            summarize_tool("Agent", &json!({"description": "Port the mapper"})),
            "Delegating: Port the mapper"
        );
        assert_eq!(summarize_tool("Task", &json!({})), "Delegating: a subtask");
        for tool in ["TodoWrite", "TaskCreate", "TaskUpdate"] {
            assert_eq!(summarize_tool(tool, &json!({})), "Updating the task list");
        }
        assert_eq!(summarize_tool("AskUserQuestion", &json!({})), "Asking you a question");
        assert_eq!(summarize_tool("ExitPlanMode", &json!({})), "Waiting for plan approval");
    }

    #[test]
    fn summarize_skill() {
        assert_eq!(summarize_tool("Skill", &json!({"skill": "pdf"})), "Using skill pdf");
        // Swift keeps the trailing space when the skill name is missing.
        assert_eq!(summarize_tool("Skill", &json!({})), "Using skill ");
    }

    #[test]
    fn summarize_mcp_tools_split_readably() {
        assert_eq!(
            summarize_tool("mcp__github__create_issue", &json!({})),
            "Using github create issue"
        );
        assert_eq!(summarize_tool("mcp__linear__search", &json!({})), "Using linear search");
        assert_eq!(summarize_tool("mcp__", &json!({})), "Using an MCP tool");
        assert_eq!(summarize_tool("SomeNewTool", &json!({})), "Using SomeNewTool");
    }

    #[test]
    fn summarize_handles_null_and_non_object_input() {
        assert_eq!(summarize_tool("Read", &Value::Null), "Reading a file");
        assert_eq!(summarize_tool("Bash", &json!("not an object")), "Running: a command");
        assert_eq!(summarize_tool("Read", &json!({"file_path": 42})), "Reading a file");
    }

    // MARK: truncation

    #[test]
    fn truncate_keeps_short_text() {
        assert_eq!(truncate(""), "");
        let exactly_100: String = "a".repeat(100);
        assert_eq!(truncate(&exactly_100), exactly_100);
    }

    #[test]
    fn truncate_cuts_at_100_chars_with_ellipsis() {
        let long: String = "a".repeat(101);
        let result = truncate(&long);
        assert_eq!(result.chars().count(), 100);
        assert_eq!(result, format!("{}…", "a".repeat(99)));
    }

    #[test]
    fn truncate_counts_chars_not_bytes() {
        // 100 multibyte chars: many more than 100 bytes but exactly 100 chars → untouched.
        let hundred_multibyte: String = "é".repeat(100);
        assert_eq!(truncate(&hundred_multibyte), hundred_multibyte);
        let over: String = "é".repeat(101);
        let result = truncate(&over);
        assert_eq!(result.chars().count(), 100);
        assert_eq!(result, format!("{}…", "é".repeat(99)));
    }

    #[test]
    fn summarize_truncates_long_lines() {
        let long_pattern = "x".repeat(200);
        let result = summarize_tool("Grep", &json!({ "pattern": long_pattern }));
        assert_eq!(result.chars().count(), 100);
        assert!(result.starts_with("Searching for “"));
        assert!(result.ends_with('…'));
    }

    // MARK: merge policy

    fn blocked_snapshot(state: PetState, pending: Option<&str>) -> SessionSnapshot {
        SessionSnapshot {
            session_id: "s1".into(),
            cwd: "/tmp/project".into(),
            state,
            message: "Approve?".into(),
            last_event_name: "PermissionRequest".into(),
            tool_name: Some("Bash".into()),
            updated_at_epoch_seconds: now_epoch_secs(),
            pending_tool_use_id: pending.map(str::to_string),
            active_agents: Vec::new(),
        }
    }

    #[test]
    fn resolve_keeps_a_finished_result_through_idle_prompt() {
        let idle_prompt = input(json!({
            "hook_event_name": "Notification",
            "session_id": "s1",
            "cwd": "/tmp/project",
            "notification_type": "idle_prompt",
            "message": "Claude is waiting for your input"
        }));
        for state in [PetState::Done, PetState::Error] {
            let mut existing = blocked_snapshot(state, None);
            existing.last_event_name = "Stop".into();
            match resolve(Some(&existing), &idle_prompt, PetState::Idle, "", None, now_epoch_secs()) {
                Resolution::Keep(reason) => assert!(reason.contains("idle_prompt"), "{reason}"),
                other => panic!("idle_prompt must not clear {state:?} (got {other:?})"),
            }
        }
        // A busy state is still cleared (interrupt cleanup).
        let busy = blocked_snapshot(PetState::Working, None);
        assert!(matches!(
            resolve(Some(&busy), &idle_prompt, PetState::Idle, "", None, now_epoch_secs()),
            Resolution::Write(_)
        ));
    }

    #[test]
    fn resolve_writes_when_no_existing_snapshot() {
        let event = input(json!({
            "hook_event_name": "PreToolUse",
            "session_id": "s1",
            "cwd": "/tmp/project",
            "tool_name": "Read"
        }));
        let now = 1_755_500_000.0;
        match resolve(None, &event, PetState::Working, "Reading main.rs", Some("Read"), now) {
            Resolution::Write(snapshot) => {
                assert_eq!(snapshot.session_id, "s1");
                assert_eq!(snapshot.cwd, "/tmp/project");
                assert_eq!(snapshot.state, PetState::Working);
                assert_eq!(snapshot.message, "Reading main.rs");
                assert_eq!(snapshot.last_event_name, "PreToolUse");
                assert_eq!(snapshot.tool_name.as_deref(), Some("Read"));
                assert_eq!(snapshot.updated_at_epoch_seconds, now);
                assert_eq!(snapshot.pending_tool_use_id, None);
            }
            other => panic!("expected write, got {other:?}"),
        }
    }

    #[test]
    fn resolve_keeps_subagent_chatter_while_blocked() {
        let existing = blocked_snapshot(PetState::WaitingApproval, Some("toolu_1"));
        let event = input(json!({
            "hook_event_name": "PostToolUse",
            "session_id": "s1",
            "agent_id": "agent-123",
            "tool_use_id": "toolu_other"
        }));
        // The event is still ignored — but it did tell us an agent is alive, so the roster
        // is written while the prompt on screen stays exactly as it was.
        match resolve(Some(&existing), &event, PetState::Thinking, "Thinking…", None, now_epoch_secs()) {
            Resolution::RosterOnly(snapshot) => {
                assert_eq!(snapshot.state, PetState::WaitingApproval);
                assert_eq!(snapshot.message, existing.message);
                assert_eq!(snapshot.pending_tool_use_id.as_deref(), Some("toolu_1"));
                assert_eq!(snapshot.active_agents.len(), 1);
            }
            other => panic!("expected a roster-only write, got {other:?}"),
        }
    }

    #[test]
    fn resolve_keeps_sibling_tool_completion_while_blocked() {
        for event_name in ["PostToolUse", "PostToolUseFailure"] {
            let existing = blocked_snapshot(PetState::NeedsInput, Some("toolu_pending"));
            let event = input(json!({
                "hook_event_name": event_name,
                "session_id": "s1",
                "tool_use_id": "toolu_sibling"
            }));
            assert_eq!(
                resolve(Some(&existing), &event, PetState::Thinking, "Thinking…", None, now_epoch_secs()),
                Resolution::Keep(
                    "sibling tool toolu_sibling finished while waiting on toolu_pending".to_string()
                )
            );
        }
    }

    #[test]
    fn resolve_writes_when_awaited_tool_finishes() {
        let existing = blocked_snapshot(PetState::WaitingApproval, Some("toolu_1"));
        let event = input(json!({
            "hook_event_name": "PostToolUse",
            "session_id": "s1",
            "tool_use_id": "toolu_1"
        }));
        match resolve(Some(&existing), &event, PetState::Thinking, "Thinking…", None, now_epoch_secs()) {
            Resolution::Write(snapshot) => {
                assert_eq!(snapshot.state, PetState::Thinking);
                // Not attention-needed → no pending id carried.
                assert_eq!(snapshot.pending_tool_use_id, None);
            }
            other => panic!("expected write, got {other:?}"),
        }
    }

    #[test]
    fn resolve_writes_tool_completion_without_ids_while_blocked() {
        // Blocked, PostToolUse, but either id missing → falls through to write.
        let existing = blocked_snapshot(PetState::WaitingApproval, None);
        let event = input(json!({
            "hook_event_name": "PostToolUse",
            "session_id": "s1",
            "tool_use_id": "toolu_x"
        }));
        assert!(matches!(
            resolve(Some(&existing), &event, PetState::Thinking, "Thinking…", None, now_epoch_secs()),
            Resolution::Write(_)
        ));
        let existing_with_pending = blocked_snapshot(PetState::WaitingApproval, Some("toolu_1"));
        let no_id_event = input(json!({"hook_event_name": "PostToolUse", "session_id": "s1"}));
        assert!(matches!(
            resolve(
                Some(&existing_with_pending),
                &no_id_event,
                PetState::Thinking,
                "Thinking…",
                None,
                now_epoch_secs()
            ),
            Resolution::Write(_)
        ));
    }

    #[test]
    fn resolve_takes_pending_id_from_input() {
        let event = input(json!({
            "hook_event_name": "PermissionRequest",
            "session_id": "s1",
            "tool_use_id": "toolu_9",
            "tool_name": "Bash"
        }));
        match resolve(None, &event, PetState::WaitingApproval, "Approve? Running: ls", Some("Bash"), now_epoch_secs()) {
            Resolution::Write(snapshot) => {
                assert_eq!(snapshot.pending_tool_use_id.as_deref(), Some("toolu_9"));
            }
            other => panic!("expected write, got {other:?}"),
        }
    }

    #[test]
    fn resolve_inherits_pending_id_when_blocked_notification_reasserts() {
        let existing = blocked_snapshot(PetState::WaitingApproval, Some("toolu_1"));
        let event = input(json!({
            "hook_event_name": "Notification",
            "session_id": "s1"
        }));
        match resolve(
            Some(&existing),
            &event,
            PetState::WaitingApproval,
            "Needs your approval",
            None,
            now_epoch_secs(),
        ) {
            Resolution::Write(snapshot) => {
                assert_eq!(snapshot.pending_tool_use_id.as_deref(), Some("toolu_1"));
            }
            other => panic!("expected write, got {other:?}"),
        }
    }

    #[test]
    fn resolve_does_not_inherit_pending_id_when_not_blocked() {
        let mut existing = blocked_snapshot(PetState::Working, Some("toolu_1"));
        existing.state = PetState::Working;
        let event = input(json!({
            "hook_event_name": "Notification",
            "session_id": "s1"
        }));
        match resolve(
            Some(&existing),
            &event,
            PetState::WaitingApproval,
            "Needs your approval",
            None,
            now_epoch_secs(),
        ) {
            Resolution::Write(snapshot) => {
                assert_eq!(snapshot.pending_tool_use_id, None);
            }
            other => panic!("expected write, got {other:?}"),
        }
    }

    #[test]
    fn resolve_uses_effective_state_so_stale_blocks_do_not_keep() {
        // waiting_approval decays to idle after 20 minutes → subagent events write again.
        let mut existing = blocked_snapshot(PetState::WaitingApproval, Some("toolu_1"));
        existing.updated_at_epoch_seconds = now_epoch_secs() - 21.0 * 60.0;
        let event = input(json!({
            "hook_event_name": "PostToolUse",
            "session_id": "s1",
            "agent_id": "agent-123",
            "tool_use_id": "toolu_other"
        }));
        assert!(matches!(
            resolve(Some(&existing), &event, PetState::Thinking, "Thinking…", None, now_epoch_secs()),
            Resolution::Write(_)
        ));
    }

    /// Drives the real `resolve` rather than the helper: the first version of these tests
    /// called `agents_after` directly and so never noticed that every `Keep` path skipped
    /// the roster entirely — which is exactly where `SubagentStop` arrives.
    fn roster_after(
        existing: Option<&SessionSnapshot>,
        event_name: &str,
        agent_id: Option<&str>,
        now: f64,
    ) -> Vec<String> {
        let mut object = json!({"hook_event_name": event_name, "session_id": "s1", "cwd": "/tmp"});
        if let Some(agent_id) = agent_id {
            object.as_object_mut().unwrap().insert("agent_id".into(), json!(agent_id));
        }
        let event = input(object);
        let snapshot = match resolve(existing, &event, PetState::Thinking, "…", None, now) {
            Resolution::Write(snapshot) | Resolution::RosterOnly(snapshot) => snapshot,
            Resolution::Keep(reason) => {
                // Nothing changed about who is working; the roster is whatever it was.
                assert!(!reason.is_empty());
                return existing.map(|e| e.active_agents.iter().map(|a| a.id.clone()).collect()).unwrap_or_default();
            }
        };
        snapshot.active_agents.into_iter().map(|agent| agent.id).collect()
    }

    fn snapshot_with_agents(state: PetState, agents: &[(&str, f64)]) -> SessionSnapshot {
        let mut snapshot = blocked_snapshot(state, None);
        snapshot.active_agents = agents
            .iter()
            .map(|(id, seen)| ActiveAgent { id: id.to_string(), last_seen_epoch_seconds: *seen })
            .collect();
        snapshot
    }

    #[test]
    fn roster_tracks_who_is_working() {
        let now = 1_000.0;
        assert_eq!(roster_after(None, "SubagentStart", Some("a1"), now), vec!["a1"]);

        let one = snapshot_with_agents(PetState::Working, &[("a1", now)]);
        assert_eq!(roster_after(Some(&one), "SubagentStart", Some("a2"), now), vec!["a1", "a2"]);
        // An agent we never saw start still counts: its tool calls give it away.
        assert_eq!(roster_after(Some(&one), "PreToolUse", Some("a9"), now), vec!["a1", "a9"]);
        // Seeing the same one again does not double it.
        assert_eq!(roster_after(Some(&one), "PostToolUse", Some("a1"), now), vec!["a1"]);
        // Events with no agent leave the roster alone.
        assert_eq!(roster_after(Some(&one), "PreToolUse", None, now), vec!["a1"]);

        let two = snapshot_with_agents(PetState::Working, &[("a1", now), ("a2", now)]);
        assert_eq!(roster_after(Some(&two), "SubagentStop", Some("a1"), now), vec!["a2"]);

        // Turn boundaries retire everyone.
        for event_name in ["UserPromptSubmit", "Stop", "StopFailure", "SessionStart"] {
            assert!(
                roster_after(Some(&two), event_name, None, now).is_empty(),
                "{event_name} should clear the roster"
            );
        }
    }

    #[test]
    fn roster_survives_the_paths_that_ignore_the_event() {
        // A stop is sent once. Every one of these states drops the event itself, and the
        // first version of this feature dropped the retirement with it — leaving a clone
        // on screen for an agent that had already reported back.
        let now = 2_000.0;
        for state in [PetState::Done, PetState::Error, PetState::WaitingApproval, PetState::NeedsInput] {
            let existing = snapshot_with_agents(state, &[("a1", now), ("a2", now)]);
            let event = input(json!({
                "hook_event_name": "SubagentStop",
                "session_id": "s1",
                "agent_id": "a1"
            }));
            match resolve(Some(&existing), &event, PetState::Thinking, "…", None, now) {
                Resolution::RosterOnly(snapshot) => {
                    assert_eq!(snapshot.state, state, "{state:?} must not be overwritten");
                    assert_eq!(snapshot.message, existing.message, "the result text stays");
                    assert_eq!(
                        snapshot.updated_at_epoch_seconds, existing.updated_at_epoch_seconds,
                        "a roster update must not restart the decay clock"
                    );
                    let ids: Vec<&str> = snapshot.active_agents.iter().map(|a| a.id.as_str()).collect();
                    assert_eq!(ids, vec!["a2"], "the agent that stopped is gone");
                }
                other => panic!("{state:?}: expected a roster-only write, got {other:?}"),
            }
        }
    }

    #[test]
    fn compaction_does_not_retire_agents_mid_turn() {
        // SessionStart also fires after auto-compaction, with the agents still working.
        let now = 3_000.0;
        let two = snapshot_with_agents(PetState::Working, &[("a1", now), ("a2", now)]);
        let compact = input(json!({
            "hook_event_name": "SessionStart",
            "session_id": "s1",
            "source": "compact"
        }));
        match resolve(Some(&two), &compact, PetState::Thinking, "…", None, now) {
            Resolution::Write(snapshot) => assert_eq!(snapshot.active_agents.len(), 2),
            other => panic!("expected a write, got {other:?}"),
        }
    }

    #[test]
    fn agents_nobody_has_heard_from_stop_counting() {
        // The self-healing half: a stop that never arrives (dropped, raced, or never sent)
        // expires instead of haunting the pet until the next prompt.
        let now = 4_000.0;
        let window = SessionSnapshot::AGENT_ALIVE_WINDOW_SECS;
        let mixed = snapshot_with_agents(
            PetState::Working,
            &[("stale", now - window - 1.0), ("fresh", now - 1.0)],
        );
        assert_eq!(mixed.live_agent_count(now), 1, "only the fresh one counts");
        assert_eq!(roster_after(Some(&mixed), "PreToolUse", Some("fresh"), now), vec!["fresh"]);
    }

    #[test]
    fn the_roster_is_bounded_and_drops_the_stalest() {
        let now = 5_000.0;
        let mut agents: Vec<(String, f64)> = (0..SessionSnapshot::MAXIMUM_TRACKED_AGENTS)
            .map(|index| (format!("a{index}"), now - index as f64))
            .collect();
        // a0 is the freshest, a7 the stalest.
        let borrowed: Vec<(&str, f64)> = agents.iter().map(|(id, seen)| (id.as_str(), *seen)).collect();
        let full = snapshot_with_agents(PetState::Working, &borrowed);
        let after = roster_after(Some(&full), "SubagentStart", Some("newcomer"), now);
        assert_eq!(after.len(), SessionSnapshot::MAXIMUM_TRACKED_AGENTS);
        assert!(after.contains(&"newcomer".to_string()));
        assert!(!after.contains(&"a7".to_string()), "the stalest is dropped, not the oldest-added");
        assert!(after.contains(&"a0".to_string()), "the freshest survives");
        agents.clear();
    }

    #[test]
    fn resolve_keeps_a_finished_result_through_late_subagent_events() {
        // The real sequence from a live session: Stop at 20:30:48, SubagentStop at 20:30:50.
        // Before this rule the trailing event turned a finished session back into thinking,
        // where it sat until the busy state decayed 15 minutes later.
        for event_name in ["SubagentStop", "PostToolUse", "PostToolBatch", "PostToolUseFailure"] {
            for state in [PetState::Done, PetState::Error] {
                let mut existing = blocked_snapshot(state, None);
                existing.last_event_name = "Stop".into();
                let event = input(json!({
                    "hook_event_name": event_name,
                    "session_id": "s1",
                    "agent_id": "agent-123"
                }));
                match resolve(Some(&existing), &event, PetState::Thinking, "Agent reported back", None, now_epoch_secs()) {
                    Resolution::Keep(reason) => {
                        assert_eq!(reason, format!("subagent {event_name} while {}", state.raw()))
                    }
                    // A roster-only write is fine — it keeps the state and only records who
                    // is working — but the result itself must not change.
                    Resolution::RosterOnly(snapshot) => assert_eq!(snapshot.state, state),
                    Resolution::Write(_) => panic!("{event_name} must not un-finish {state:?}"),
                }
            }
        }
    }

    #[test]
    fn resolve_lets_a_subagent_that_starts_working_revive_a_finished_result() {
        // Background workflows keep agents running after the turn ends. Their traffic is
        // the only signal that anything is happening, so treating it as trailing chatter
        // left the pet showing done through minutes of real work.
        for (event_name, mapped, message) in [
            ("SubagentStart", PetState::Working, "Sent a reviewer agent to work"),
            ("PreToolUse", PetState::Working, "Running: cargo test"),
        ] {
            let mut existing = blocked_snapshot(PetState::Done, None);
            existing.last_event_name = "Stop".into();
            let event = input(json!({
                "hook_event_name": event_name,
                "session_id": "s1",
                "agent_id": "agent-123",
                "tool_name": "Bash"
            }));
            match resolve(Some(&existing), &event, mapped, message, None, now_epoch_secs()) {
                Resolution::Write(snapshot) => assert_eq!(snapshot.state, mapped),
                other => panic!("{event_name} is real work, not chatter (got {other:?})"),
            }
        }

        // While the user is being asked something, though, nothing from a subagent may
        // overwrite the prompt — that guard is unchanged.
        let existing = blocked_snapshot(PetState::WaitingApproval, None);
        let start = input(json!({
            "hook_event_name": "SubagentStart",
            "session_id": "s1",
            "agent_id": "agent-123"
        }));
        match resolve(Some(&existing), &start, PetState::Working, "…", None, now_epoch_secs()) {
            // Roster recorded, prompt untouched: the red clock must survive a subagent.
            Resolution::RosterOnly(snapshot) => assert_eq!(snapshot.state, PetState::WaitingApproval),
            Resolution::Keep(_) => {}
            Resolution::Write(_) => panic!("a subagent must not overwrite a prompt"),
        }
    }

    #[test]
    fn resolve_lets_real_work_replace_a_finished_result() {
        // Only subagent chatter is ignored: the user's next turn still moves the pet on.
        for (event_name, state) in [("UserPromptSubmit", PetState::Thinking), ("PreToolUse", PetState::Working)] {
            let mut existing = blocked_snapshot(PetState::Done, None);
            existing.last_event_name = "Stop".into();
            let event = input(json!({
                "hook_event_name": event_name,
                "session_id": "s1",
                "tool_name": "Read"
            }));
            assert!(
                matches!(
                    resolve(Some(&existing), &event, state, "…", None, now_epoch_secs()),
                    Resolution::Write(_)
                ),
                "{event_name} should replace a finished result"
            );
        }
    }

    #[test]
    fn resolve_subagent_writes_when_not_blocked() {
        let mut existing = blocked_snapshot(PetState::Thinking, None);
        existing.state = PetState::Thinking;
        let event = input(json!({
            "hook_event_name": "PreToolUse",
            "session_id": "s1",
            "agent_id": "agent-123",
            "tool_name": "Read"
        }));
        assert!(matches!(
            resolve(Some(&existing), &event, PetState::Working, "Reading x", Some("Read"), now_epoch_secs()),
            Resolution::Write(_)
        ));
    }

    #[test]
    fn hook_input_parse_rejects_non_objects() {
        assert!(HookInput::parse(b"[1,2,3]").is_none());
        assert!(HookInput::parse(b"not json").is_none());
        assert!(HookInput::parse(b"").is_none());
        assert!(HookInput::parse(b"{}").is_some());
    }

    #[test]
    fn hook_input_defaults() {
        let parsed = HookInput::parse(b"{}").unwrap();
        assert_eq!(parsed.session_id(), "unknown-session");
        assert_eq!(parsed.hook_event_name(), "");
        assert!(!parsed.is_subagent_event());
        assert!(parsed.tool_input().is_null());
    }
}
