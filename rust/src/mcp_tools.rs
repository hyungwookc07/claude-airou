//! The MCP tools offered to Claude chat. Port of `MCP/MCPPetTools.swift` — tool names,
//! JSON schemas, description texts and result texts should match the Swift server so both
//! binaries feel identical from inside a chat.
//!
//! Tools: pet_status (state enum thinking/working/needs_input/done/error/idle/hello +
//! optional message ≤120 chars, defaults per state), list_pets, preview_pet (id → sheet PNG
//! as base64 image content + text), hatch_pet (definition object → decode + validate + save
//! to ~/.claude-airou/pets/<id>.json + sheet image; validation failures return isError with
//! the fix-it hint text). Image content items: {"type":"image","data":<b64>,
//! "mimeType":"image/png"}; ASCII fallback when rendering fails.

use crate::mcp::SharedServerState;
use serde_json::Value;

pub const PET_STATUS_TOOL_NAME: &str = "pet_status";

pub struct ToolResult {
    /// MCP content items (text / image objects).
    pub content: Vec<Value>,
    pub is_error: bool,
}

impl ToolResult {
    pub fn text(text: impl Into<String>) -> ToolResult {
        ToolResult {
            content: vec![serde_json::json!({"type": "text", "text": text.into()})],
            is_error: false,
        }
    }

    pub fn failure(text: impl Into<String>) -> ToolResult {
        ToolResult {
            content: vec![serde_json::json!({"type": "text", "text": text.into()})],
            is_error: true,
        }
    }
}

/// The `tools` array for tools/list.
pub fn descriptors() -> Value {
    todo!("port MCPPetTools descriptors (pet_status, list_pets, preview_pet, hatch_pet)")
}

/// Dispatch for tools/call; `None` = unknown tool (server answers -32602).
pub fn call(name: &str, arguments: &Value, server: &SharedServerState) -> Option<ToolResult> {
    let _ = (name, arguments, server);
    todo!("port MCPPetTools.call")
}
