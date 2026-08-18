//! Pure mapping from a Claude Code hook event to what the pet should do, plus the merge
//! policy for concurrent events. Port of `Hook/HookEventMapper.swift` (incl. ToolSummarizer)
//! and `Hook/HookMergePolicy.swift` — behaviour must match 1:1.

use crate::model::{PetState, SessionSnapshot};
use serde_json::Value;

/// Events the installer registers. Anything else is ignored if it ever arrives.
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

/// Port of `HookEventMapper.map(_:)` — every branch, message string and state must match
/// the Swift original exactly (see the table in README.md).
pub fn map(input: &HookInput) -> MappingResult {
    let _ = input;
    todo!("port HookEventMapper.map from Sources/ClaudeAirou/Hook/HookEventMapper.swift")
}

/// Port of `ToolSummarizer.summarize` — one bubble line per tool call.
pub fn summarize_tool(tool_name: &str, tool_input: &Value) -> String {
    let _ = (tool_name, tool_input);
    todo!("port ToolSummarizer.summarize")
}

/// Port of `ToolSummarizer.truncate` (100 chars max, ellipsis).
pub fn truncate(text: &str) -> String {
    let _ = text;
    todo!("port ToolSummarizer.truncate")
}

#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
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
    let _ = (existing, input, mapped_state, message, tool_name, now_secs);
    todo!("port HookMergePolicy.resolve from Sources/ClaudeAirou/Hook/HookMergePolicy.swift")
}
