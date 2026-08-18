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
use crate::model::{AppConfig, PetState};
use crate::pets::{PetDefinition, PetLibrary, BUILT_IN_PET_SOURCES};
use serde_json::{json, Value};

pub const PET_STATUS_TOOL_NAME: &str = "pet_status";

/// States chat may set. `waiting_approval` is left out: chat has no permission prompts,
/// `needs_input` covers "it is the user's turn".
const SETTABLE_STATES: [PetState; 7] = [
    PetState::Thinking,
    PetState::Working,
    PetState::NeedsInput,
    PetState::Done,
    PetState::Error,
    PetState::Idle,
    PetState::Hello,
];

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

// MARK: - Descriptions (verbatim from MCPPetTools.swift; its multiline literals join
// backslash-continued lines with spaces and keep real newlines elsewhere)

const PET_STATUS_DESCRIPTION: &str = concat!(
    "Update the user's claude-airou desktop pet — a pixel companion floating on their ",
    "screen that mirrors what Claude is doing. Call it at real transitions: \"thinking\" ",
    "when you start on a request, \"working\" while running a longer step, \"done\" when you ",
    "finish, \"error\" when something fails, \"needs_input\" when you are waiting for the ",
    "user's answer, \"idle\" when nothing is pending. The optional message appears in the ",
    "pet's speech bubble — keep it under 60 characters, e.g. \"Summarizing the PDF…\"."
);

const LIST_PETS_DESCRIPTION: &str =
    "List the pets available to the claude-airou overlay (built-in and custom) and which one is selected.";

const PREVIEW_PET_DESCRIPTION: &str = concat!(
    "Render a pet's full sprite sheet as an image so you and the user can look at it. ",
    "Rows are the states in order hello, idle, thinking, working, waiting_approval, ",
    "needs_input, done, error; columns are animation frames."
);

const HATCH_PET_DESCRIPTION: &str = concat!(
    "Create (or edit) a custom pixel-art pet for the claude-airou overlay. Pass the ",
    "complete definition object; it is validated, saved to ~/.claude-airou/pets/<id>.json ",
    "and the rendered sprite sheet comes back so you can judge it and iterate. Format:\n",
    "{\"id\":\"nori-axolotl\",\"name\":\"Nori\",\"species\":\"axolotl\",\"fps\":3,\n",
    " \"palette\":{\"k\":\"#3a2a2a\",\"p\":\"#f6a7c1\",\"w\":\"#ffffff\",\"e\":\"#222222\"},\n",
    " \"phrases\":{\"pet\":[\"blub.\"]},\n",
    " \"frames\":{\"idle\":[[\"..kk..\",\"..pp..\"],[\"..kk..\",\"..pp..\"]],\"thinking\":[…],\"working\":[…],\n",
    "           \"waiting_approval\":[…],\"needs_input\":[…],\"done\":[…],\"error\":[…],\"hello\":[…]}}\n",
    "Rules: each state maps to an array of frames; a frame is an array of equally long row ",
    "strings and every frame in every state shares one grid size (16×16–24×24 works best, ",
    "min 4, max 64). Characters are single-character palette keys (\"#RRGGBB\" or ",
    "\"#RRGGBBAA\" values); \".\" and space are transparent. \"frames.idle\" is required; missing ",
    "states fall back (working→thinking→idle, hello→done→idle, waiting_approval↔needs_input). ",
    "Design: 4–8 colours with one dark outline; keep the body identical across states and ",
    "change only eyes/mouth/small props (eyes up + blue dots for thinking, focused eyes for ",
    "working, wide eyes for waiting_approval, ^ ^ eyes + sparkle for done, x x eyes for ",
    "error, one paw raised for hello); idle gets 2–4 frames with subtle motion like a ",
    "blink. The overlay draws its own status badges, so sprites only change expression. ",
    "After hatching, check the sheet: silhouette readable, eyes visible, states distinct — ",
    "call hatch_pet again with a fixed definition to iterate."
);

// MARK: - Descriptors (tools/list)

/// The `tools` array for tools/list.
pub fn descriptors() -> Value {
    let settable: Vec<&str> = SETTABLE_STATES.iter().map(|state| state.raw()).collect();
    json!([
        {
            "name": PET_STATUS_TOOL_NAME,
            "description": PET_STATUS_DESCRIPTION,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "state": {
                        "type": "string",
                        "description": "What the pet should show.",
                        "enum": settable,
                    },
                    "message": {
                        "type": "string",
                        "description": "Optional speech-bubble text (short).",
                    },
                },
                "required": ["state"],
            },
        },
        {
            "name": "list_pets",
            "description": LIST_PETS_DESCRIPTION,
            "inputSchema": {"type": "object", "properties": {}},
        },
        {
            "name": "preview_pet",
            "description": PREVIEW_PET_DESCRIPTION,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "Pet id from list_pets, e.g. \"mochi-cat\".",
                    },
                },
                "required": ["id"],
            },
        },
        {
            "name": "hatch_pet",
            "description": HATCH_PET_DESCRIPTION,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "definition": {
                        "type": "object",
                        "description": "The full pet definition JSON object (see the tool description for the format).",
                    },
                },
                "required": ["definition"],
            },
        },
    ])
}

// MARK: - Dispatch (tools/call)

/// Dispatch for tools/call; `None` = unknown tool (server answers -32602).
pub fn call(name: &str, arguments: &Value, server: &SharedServerState) -> Option<ToolResult> {
    match name {
        PET_STATUS_TOOL_NAME => Some(pet_status(arguments, server)),
        "list_pets" => Some(list_pets()),
        "preview_pet" => Some(preview_pet(arguments)),
        "hatch_pet" => Some(hatch_pet(arguments)),
        _ => None,
    }
}

// MARK: - pet_status

fn pet_status(arguments: &Value, server: &SharedServerState) -> ToolResult {
    let Some(state) = arguments
        .get("state")
        .and_then(Value::as_str)
        .and_then(PetState::parse)
    else {
        let allowed = SETTABLE_STATES
            .iter()
            .map(|state| state.raw())
            .collect::<Vec<_>>()
            .join(", ");
        return ToolResult::failure(format!("`state` must be one of: {allowed}"));
    };
    let mut message = arguments
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if message.chars().count() > 120 {
        message = message.chars().take(119).collect::<String>() + "…";
    }
    if message.is_empty() {
        message = default_message(state).to_string();
    }
    crate::mcp::write_state(server, state, &message, "mcp:pet_status");
    let clause = if message.is_empty() {
        String::new()
    } else {
        format!(" — “{message}”")
    };
    ToolResult::text(format!(
        "The pet now shows \"{}\"{clause}. Update it again at the next real transition.",
        state.raw()
    ))
}

fn default_message(state: PetState) -> &'static str {
    match state {
        PetState::Hello => "Hi! Ready when you are",
        PetState::Idle => "",
        PetState::Thinking => "Thinking…",
        PetState::Working => "Working on it…",
        PetState::WaitingApproval => "Waiting for approval",
        PetState::NeedsInput => "Your turn!",
        PetState::Done => "Done!",
        PetState::Error => "Something failed — recovering…",
    }
}

// MARK: - list_pets

fn list_pets() -> ToolResult {
    let library = PetLibrary::load();
    let config = AppConfig::load();
    let selected_id = library
        .resolve_selected(config.selected_pet_id.as_deref())
        .map(|pet| pet.definition.id.clone());
    let mut lines: Vec<String> = Vec::new();
    for loaded in &library.pets {
        let definition = &loaded.definition;
        let (width, height) = definition.grid_size();
        let origin = if loaded.is_built_in() { "built-in" } else { "custom" };
        let marker = if Some(&definition.id) == selected_id.as_ref() {
            " ← selected"
        } else {
            ""
        };
        lines.push(format!(
            "{} — {} the {} ({width}x{height}, {origin}){marker}",
            definition.id, definition.name, definition.species
        ));
    }
    for problem in &library.load_problems {
        lines.push(format!("skipped: {problem}"));
    }
    lines.push(String::new());
    lines.push(
        "The user switches pets via the menu bar 🐾 → Pet (use \"Reload pets\" after hatching while the overlay is running)."
            .to_string(),
    );
    ToolResult::text(lines.join("\n"))
}

// MARK: - preview_pet

fn preview_pet(arguments: &Value) -> ToolResult {
    let id = arguments.get("id").and_then(Value::as_str).unwrap_or("");
    if id.is_empty() {
        return ToolResult::failure("`id` is required (see list_pets)");
    }
    let library = PetLibrary::load();
    let Some(loaded) = library.pet_with_id(id) else {
        let known = library
            .pets
            .iter()
            .map(|pet| pet.definition.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return ToolResult::failure(format!("No pet with id \"{id}\". Available: {known}"));
    };
    sheet_result(
        &loaded.definition,
        &format!(
            "{} the {} ({id}). Rows top to bottom: hello, idle, thinking, working, waiting_approval, needs_input, done, error; columns are frames.",
            loaded.definition.name, loaded.definition.species
        ),
    )
}

// MARK: - hatch_pet

fn hatch_pet(arguments: &Value) -> ToolResult {
    let Some(raw_definition) = arguments.get("definition").filter(|value| value.is_object()) else {
        return ToolResult::failure("`definition` must be the full pet JSON object (not a string).");
    };

    let definition: PetDefinition = match serde_json::from_value(raw_definition.clone()) {
        Ok(definition) => definition,
        Err(error) => return ToolResult::failure(format!("Invalid pet JSON structure: {error}")),
    };

    let warnings = match definition.validate() {
        Ok(warnings) => warnings,
        Err(error) => {
            return ToolResult::failure(format!(
                "Validation failed:\n{error}\n\nUsual culprit: rows with mismatched widths — count the characters of every row; all frames of all states must share one grid size. Fix the definition and call hatch_pet again."
            ));
        }
    };

    let pets_dir = crate::paths::pets_dir();
    let file_path = pets_dir.join(format!("{}.json", definition.id));
    let replaced_existing = file_path.exists();
    let saved = crate::paths::ensure_dir(&pets_dir)
        .map_err(|error| error.to_string())
        .and_then(|_| serde_json::to_vec_pretty(&definition).map_err(|error| error.to_string()))
        .and_then(|data| {
            crate::state_store::write_atomic(&file_path, &data).map_err(|error| error.to_string())
        });
    if let Err(error) = saved {
        return ToolResult::failure(format!("Could not save {}: {error}", file_path.display()));
    }

    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "Hatched {} the {} → {}{}",
        definition.name,
        definition.species,
        file_path.display(),
        if replaced_existing { " (replaced the previous version)" } else { "" }
    ));
    if built_in_pet_ids().iter().any(|id| id == &definition.id) {
        lines.push(format!(
            "Note: this id shadows the built-in \"{}\" until the file is deleted.",
            definition.id
        ));
    }
    for warning in &warnings {
        lines.push(format!("warning: {warning}"));
    }
    lines.push(format!(
        "Pick it via the menu bar 🐾 → Pet → {} (\"Reload pets\" first if the overlay is already running).",
        definition.name
    ));
    lines.push(
        "Check the sheet below — silhouette readable? eyes visible? states distinct? Iterate with hatch_pet if not."
            .to_string(),
    );
    sheet_result(&definition, &lines.join("\n"))
}

fn built_in_pet_ids() -> Vec<String> {
    BUILT_IN_PET_SOURCES
        .iter()
        .filter_map(|(_, source)| PetDefinition::decode(source.as_bytes()).ok())
        .map(|definition| definition.id)
        .collect()
}

// MARK: - Sheet rendering

/// Renders the contact sheet and returns it inline as image content (plus `text`).
/// Falls back to ASCII if PNG rendering fails.
fn sheet_result(definition: &PetDefinition, text: &str) -> ToolResult {
    match crate::render::sheet_png_bytes(definition, 8) {
        Ok(png) => ToolResult {
            content: vec![
                json!({"type": "text", "text": text}),
                json!({"type": "image", "data": base64_encode(&png), "mimeType": "image/png"}),
            ],
            is_error: false,
        },
        Err(error) => {
            let ascii = definition
                .frames_for(PetState::Idle)
                .first()
                .map(|frame| crate::render::ascii_art(frame, false))
                .unwrap_or_default();
            ToolResult::text(format!(
                "{text}\n(rendering the sheet failed: {error})\nASCII idle frame:\n{ascii}"
            ))
        }
    }
}

/// RFC 4648 standard-alphabet base64 with padding (what Swift's
/// `Data.base64EncodedString()` produces). Hand-rolled: no new dependencies.
fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(triple >> 18 & 63) as usize] as char);
        out.push(ALPHABET[(triple >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(triple >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(triple & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::ServerState;
    use crate::model::now_epoch_secs;
    use crate::state_store::StateStore;
    use std::sync::{Arc, Mutex};

    fn shared_with_store(directory: &std::path::Path) -> SharedServerState {
        Arc::new(Mutex::new(ServerState {
            store: StateStore::new(directory.to_path_buf()),
            session_id: "claude-chat-test".to_string(),
            session_label: "Claude Chat".to_string(),
            last_written_state: None,
            last_write_epoch_secs: now_epoch_secs(),
        }))
    }

    fn text_of(result: &ToolResult) -> &str {
        result.content[0]["text"].as_str().unwrap()
    }

    #[test]
    fn base64_rfc4648_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_binary_vectors() {
        assert_eq!(base64_encode(&[0xFF]), "/w==");
        assert_eq!(base64_encode(&[0xFF, 0xEE]), "/+4=");
        assert_eq!(base64_encode(&[0x00, 0x00, 0x00]), "AAAA");
        assert_eq!(base64_encode(&[0xFB]), "+w==");
        // PNG signature, a realistic prefix.
        assert_eq!(
            base64_encode(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]),
            "iVBORw0KGgo="
        );
        // Length is always a multiple of four.
        let all_bytes: Vec<u8> = (0u16..=255).map(|byte| byte as u8).collect();
        let encoded = base64_encode(&all_bytes);
        assert_eq!(encoded.len(), 256usize.div_ceil(3) * 4);
        assert!(encoded.ends_with('='));
    }

    #[test]
    fn descriptors_match_swift_shapes() {
        let descriptors = descriptors();
        let tools = descriptors.as_array().unwrap();
        assert_eq!(tools.len(), 4);

        let pet_status = &tools[0];
        assert_eq!(pet_status["name"], "pet_status");
        assert_eq!(
            pet_status["inputSchema"]["properties"]["state"]["enum"],
            json!(["thinking", "working", "needs_input", "done", "error", "idle", "hello"])
        );
        assert_eq!(pet_status["inputSchema"]["required"], json!(["state"]));
        assert_eq!(
            pet_status["inputSchema"]["properties"]["message"]["description"],
            "Optional speech-bubble text (short)."
        );
        let description = pet_status["description"].as_str().unwrap();
        assert!(description.starts_with("Update the user's claude-airou desktop pet — a pixel companion"));
        assert!(description.ends_with("e.g. \"Summarizing the PDF…\"."));

        let list_pets = &tools[1];
        assert_eq!(list_pets["name"], "list_pets");
        assert_eq!(list_pets["description"], LIST_PETS_DESCRIPTION);
        // objectSchema() with no required keys: only type + properties.
        assert_eq!(list_pets["inputSchema"], json!({"type": "object", "properties": {}}));

        let preview_pet = &tools[2];
        assert_eq!(preview_pet["name"], "preview_pet");
        assert_eq!(preview_pet["inputSchema"]["required"], json!(["id"]));
        assert_eq!(
            preview_pet["description"],
            "Render a pet's full sprite sheet as an image so you and the user can look at it. Rows are the states in order hello, idle, thinking, working, waiting_approval, needs_input, done, error; columns are animation frames."
        );

        let hatch_pet = &tools[3];
        assert_eq!(hatch_pet["name"], "hatch_pet");
        assert_eq!(hatch_pet["inputSchema"]["required"], json!(["definition"]));
        let hatch_description = hatch_pet["description"].as_str().unwrap();
        assert!(hatch_description.starts_with("Create (or edit) a custom pixel-art pet"));
        // The example JSON block keeps its real newlines and indentation.
        assert!(hatch_description.contains("iterate. Format:\n{\"id\":\"nori-axolotl\""));
        assert!(hatch_description.contains("\n \"palette\":{\"k\":\"#3a2a2a\""));
        assert!(hatch_description.contains("\n           \"waiting_approval\":[…]"));
        assert!(hatch_description.contains("}}\nRules: each state maps to an array of frames"));
        assert!(hatch_description.ends_with("call hatch_pet again with a fixed definition to iterate."));
    }

    #[test]
    fn unknown_tool_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_with_store(dir.path());
        assert!(call("bogus", &json!({}), &shared).is_none());
        assert!(call("", &json!({}), &shared).is_none());
    }

    #[test]
    fn pet_status_accepts_aliases_like_the_swift_parser() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_with_store(dir.path());
        // Swift guards on PetState.parse, not on settableStates: aliases pass through.
        let result = call("pet_status", &json!({"state": "busy"}), &shared).unwrap();
        assert!(!result.is_error);
        assert_eq!(
            text_of(&result),
            "The pet now shows \"working\" — “Working on it…”. Update it again at the next real transition."
        );
        // Even waiting_approval, though the schema enum omits it.
        let result = call("pet_status", &json!({"state": "waiting_approval"}), &shared).unwrap();
        assert!(!result.is_error);
        assert_eq!(
            text_of(&result),
            "The pet now shows \"waiting_approval\" — “Waiting for approval”. Update it again at the next real transition."
        );
        let snapshot = shared.lock().unwrap().store.read("claude-chat-test").unwrap();
        assert_eq!(snapshot.state, PetState::WaitingApproval);
        assert_eq!(snapshot.message, "Waiting for approval");
    }

    #[test]
    fn pet_status_truncates_messages_over_120_chars() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_with_store(dir.path());
        let long = "a".repeat(121);
        let result = call("pet_status", &json!({"state": "thinking", "message": long}), &shared).unwrap();
        let expected = format!("{}…", "a".repeat(119));
        assert_eq!(expected.chars().count(), 120);
        assert!(text_of(&result).contains(&format!("“{expected}”")));
        let snapshot = shared.lock().unwrap().store.read("claude-chat-test").unwrap();
        assert_eq!(snapshot.message, expected);

        // Exactly 120 characters is left alone.
        let borderline = "b".repeat(120);
        let result =
            call("pet_status", &json!({"state": "thinking", "message": borderline}), &shared).unwrap();
        assert!(text_of(&result).contains(&format!("“{}”", "b".repeat(120))));
    }

    #[test]
    fn pet_status_non_string_state_fails_with_allowed_list() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_with_store(dir.path());
        for arguments in [json!({}), json!({"state": 5}), json!({"state": "sleepy"})] {
            let result = call("pet_status", &arguments, &shared).unwrap();
            assert!(result.is_error);
            assert_eq!(
                text_of(&result),
                "`state` must be one of: thinking, working, needs_input, done, error, idle, hello"
            );
        }
        assert!(shared.lock().unwrap().store.read("claude-chat-test").is_none());
    }

    #[test]
    fn default_messages_match_swift() {
        assert_eq!(default_message(PetState::Hello), "Hi! Ready when you are");
        assert_eq!(default_message(PetState::Idle), "");
        assert_eq!(default_message(PetState::Thinking), "Thinking…");
        assert_eq!(default_message(PetState::Working), "Working on it…");
        assert_eq!(default_message(PetState::WaitingApproval), "Waiting for approval");
        assert_eq!(default_message(PetState::NeedsInput), "Your turn!");
        assert_eq!(default_message(PetState::Done), "Done!");
        assert_eq!(default_message(PetState::Error), "Something failed — recovering…");
    }

    #[test]
    fn preview_pet_requires_id_and_lists_known_ids() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_with_store(dir.path());
        let result = call("preview_pet", &json!({}), &shared).unwrap();
        assert!(result.is_error);
        assert_eq!(text_of(&result), "`id` is required (see list_pets)");

        let result = call("preview_pet", &json!({"id": "snorlax"}), &shared).unwrap();
        assert!(result.is_error);
        let text = text_of(&result);
        assert!(text.starts_with("No pet with id \"snorlax\". Available: "));
        assert!(text.contains("airou-felyne"));
        assert!(text.contains("clawd-claude"));
    }

    #[test]
    fn hatch_pet_rejects_non_object_definition() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_with_store(dir.path());
        for arguments in [
            json!({}),
            json!({"definition": "{\"id\":\"x\"}"}),
            json!({"definition": 4}),
            json!({"definition": ["not", "an", "object"]}),
        ] {
            let result = call("hatch_pet", &arguments, &shared).unwrap();
            assert!(result.is_error);
            assert_eq!(
                text_of(&result),
                "`definition` must be the full pet JSON object (not a string)."
            );
        }
    }

    #[test]
    fn hatch_pet_reports_decode_errors() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_with_store(dir.path());
        // Missing required keys (name, species, palette, frames).
        let result = call("hatch_pet", &json!({"definition": {"id": "half-pet"}}), &shared).unwrap();
        assert!(result.is_error);
        assert!(text_of(&result).starts_with("Invalid pet JSON structure: "));
    }

    #[test]
    fn hatch_pet_reports_validation_failure_with_hint() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_with_store(dir.path());
        let definition = json!({
            "id": "wonky",
            "name": "Wonky",
            "species": "blob",
            "palette": {"k": "#112233"},
            "frames": {"idle": [["kkkk", "kkk", "kkkk", "kkkk"]]},
        });
        let result = call("hatch_pet", &json!({"definition": definition}), &shared).unwrap();
        assert!(result.is_error);
        let text = text_of(&result);
        assert!(text.starts_with("Validation failed:\n"));
        assert!(text.contains("frames.idle[0] row 1 has 3 columns, expected 4"));
        assert!(text.ends_with(
            "Usual culprit: rows with mismatched widths — count the characters of every row; all frames of all states must share one grid size. Fix the definition and call hatch_pet again."
        ));
    }

    #[test]
    fn built_in_ids_cover_the_embedded_pets() {
        let ids = built_in_pet_ids();
        assert_eq!(ids.len(), 8);
        assert!(ids.contains(&"airou-felyne".to_string()));
        assert!(ids.contains(&"mochi-cat".to_string()));
    }
}
