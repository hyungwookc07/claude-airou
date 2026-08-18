import Foundation

/// The tools `claude-airou mcp` offers to Claude chat. Their descriptions double as the
/// "skill": chat sessions cannot read skills/hatch-pet/SKILL.md, so the pet-format rules
/// live in the `hatch_pet` description and results carry the rendered sheet to iterate on.
enum MCPPetTools {
    static let petStatusToolName = "pet_status"

    struct ToolResult {
        var content: [[String: Any]]
        var isError = false

        static func text(_ text: String) -> ToolResult {
            ToolResult(content: [["type": "text", "text": text]])
        }

        static func failure(_ text: String) -> ToolResult {
            ToolResult(content: [["type": "text", "text": text]], isError: true)
        }
    }

    /// States chat may set. `waiting_approval` is left out: chat has no permission prompts,
    /// `needs_input` covers "it is the user's turn".
    static let settableStates: [PetState] = [.thinking, .working, .needsInput, .done, .error, .idle, .hello]

    // MARK: - Descriptors (tools/list)

    static var descriptors: [[String: Any]] {
        [petStatusDescriptor, listPetsDescriptor, previewPetDescriptor, hatchPetDescriptor]
    }

    /// JSON Schema builders, so the descriptors below stay free of heterogeneous-literal noise.
    private static func objectSchema(properties: [String: Any] = [:], required: [String] = []) -> [String: Any] {
        var schema: [String: Any] = ["type": "object", "properties": properties]
        if !required.isEmpty { schema["required"] = required }
        return schema
    }

    private static func property(type: String, description: String, enumValues: [String]? = nil) -> [String: Any] {
        var property: [String: Any] = ["type": type, "description": description]
        if let enumValues { property["enum"] = enumValues }
        return property
    }

    private static var petStatusDescriptor: [String: Any] {
        [
            "name": petStatusToolName,
            "description": """
            Update the user's claude-airou desktop pet — a pixel companion floating on their \
            screen that mirrors what Claude is doing. Call it at real transitions: "thinking" \
            when you start on a request, "working" while running a longer step, "done" when you \
            finish, "error" when something fails, "needs_input" when you are waiting for the \
            user's answer, "idle" when nothing is pending. The optional message appears in the \
            pet's speech bubble — keep it under 60 characters, e.g. "Summarizing the PDF…".
            """,
            "inputSchema": objectSchema(
                properties: [
                    "state": property(type: "string", description: "What the pet should show.", enumValues: settableStates.map(\.rawValue)),
                    "message": property(type: "string", description: "Optional speech-bubble text (short)."),
                ],
                required: ["state"]
            ),
        ]
    }

    private static var listPetsDescriptor: [String: Any] {
        [
            "name": "list_pets",
            "description": "List the pets available to the claude-airou overlay (built-in and custom) and which one is selected.",
            "inputSchema": objectSchema(),
        ]
    }

    private static var previewPetDescriptor: [String: Any] {
        [
            "name": "preview_pet",
            "description": """
            Render a pet's full sprite sheet as an image so you and the user can look at it. \
            Rows are the states in order hello, idle, thinking, working, waiting_approval, \
            needs_input, done, error; columns are animation frames.
            """,
            "inputSchema": objectSchema(
                properties: [
                    "id": property(type: "string", description: "Pet id from list_pets, e.g. \"mochi-cat\"."),
                ],
                required: ["id"]
            ),
        ]
    }

    private static var hatchPetDescriptor: [String: Any] {
        [
            "name": "hatch_pet",
            "description": """
            Create (or edit) a custom pixel-art pet for the claude-airou overlay. Pass the \
            complete definition object; it is validated, saved to ~/.claude-airou/pets/<id>.json \
            and the rendered sprite sheet comes back so you can judge it and iterate. Format:
            {"id":"nori-axolotl","name":"Nori","species":"axolotl","fps":3,
             "palette":{"k":"#3a2a2a","p":"#f6a7c1","w":"#ffffff","e":"#222222"},
             "phrases":{"pet":["blub."]},
             "frames":{"idle":[["..kk..","..pp.."],["..kk..","..pp.."]],"thinking":[…],"working":[…],
                       "waiting_approval":[…],"needs_input":[…],"done":[…],"error":[…],"hello":[…]}}
            Rules: each state maps to an array of frames; a frame is an array of equally long row \
            strings and every frame in every state shares one grid size (16×16–24×24 works best, \
            min 4, max 64). Characters are single-character palette keys ("#RRGGBB" or \
            "#RRGGBBAA" values); "." and space are transparent. "frames.idle" is required; missing \
            states fall back (working→thinking→idle, hello→done→idle, waiting_approval↔needs_input). \
            Design: 4–8 colours with one dark outline; keep the body identical across states and \
            change only eyes/mouth/small props (eyes up + blue dots for thinking, focused eyes for \
            working, wide eyes for waiting_approval, ^ ^ eyes + sparkle for done, x x eyes for \
            error, one paw raised for hello); idle gets 2–4 frames with subtle motion like a \
            blink. The overlay draws its own status badges, so sprites only change expression. \
            After hatching, check the sheet: silhouette readable, eyes visible, states distinct — \
            call hatch_pet again with a fixed definition to iterate.
            """,
            "inputSchema": objectSchema(
                properties: [
                    "definition": property(type: "object", description: "The full pet definition JSON object (see the tool description for the format)."),
                ],
                required: ["definition"]
            ),
        ]
    }

    // MARK: - Dispatch (tools/call)

    /// Returns nil for a tool this server does not have.
    static func call(name: String, arguments: [String: Any], server: MCPServer) -> ToolResult? {
        switch name {
        case petStatusToolName: return petStatus(arguments: arguments, server: server)
        case "list_pets": return listPets()
        case "preview_pet": return previewPet(arguments: arguments)
        case "hatch_pet": return hatchPet(arguments: arguments)
        default: return nil
        }
    }

    // MARK: - pet_status

    private static func petStatus(arguments: [String: Any], server: MCPServer) -> ToolResult {
        guard let stateText = arguments["state"] as? String, let state = PetState.parse(stateText) else {
            return .failure("`state` must be one of: \(settableStates.map(\.rawValue).joined(separator: ", "))")
        }
        var message = (arguments["message"] as? String ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        if message.count > 120 { message = String(message.prefix(119)) + "…" }
        if message.isEmpty { message = defaultMessage(for: state) }
        server.writeState(state, message: message, event: "mcp:pet_status")
        return .text("The pet now shows \"\(state.rawValue)\"\(message.isEmpty ? "" : " — “\(message)”"). Update it again at the next real transition.")
    }

    private static func defaultMessage(for state: PetState) -> String {
        switch state {
        case .hello: return "Hi! Ready when you are"
        case .idle: return ""
        case .thinking: return "Thinking…"
        case .working: return "Working on it…"
        case .waitingApproval: return "Waiting for approval"
        case .needsInput: return "Your turn!"
        case .done: return "Done!"
        case .error: return "Something failed — recovering…"
        }
    }

    // MARK: - list_pets

    private static func listPets() -> ToolResult {
        let library = PetLibrary.load()
        let config = AppConfig.load()
        let selected = library.resolveSelectedPet(preferredId: config.selectedPetId)
        var lines: [String] = []
        for loadedPet in library.pets {
            let definition = loadedPet.definition
            let grid = definition.gridSize
            let origin = loadedPet.isBuiltIn ? "built-in" : "custom"
            let marker = definition.id == selected?.definition.id ? " ← selected" : ""
            lines.append("\(definition.id) — \(definition.name) the \(definition.species) (\(grid.width)x\(grid.height), \(origin))\(marker)")
        }
        for problem in library.loadProblems {
            lines.append("skipped: \(problem)")
        }
        lines.append("")
        lines.append("The user switches pets via the menu bar 🐾 → Pet (use \"Reload pets\" after hatching while the overlay is running).")
        return .text(lines.joined(separator: "\n"))
    }

    // MARK: - preview_pet

    private static func previewPet(arguments: [String: Any]) -> ToolResult {
        guard let id = arguments["id"] as? String, !id.isEmpty else {
            return .failure("`id` is required (see list_pets)")
        }
        let library = PetLibrary.load()
        guard let loadedPet = library.pet(withId: id) else {
            let known = library.pets.map(\.definition.id).joined(separator: ", ")
            return .failure("No pet with id \"\(id)\". Available: \(known)")
        }
        return sheetResult(
            for: loadedPet.definition,
            text: "\(loadedPet.definition.name) the \(loadedPet.definition.species) (\(id)). Rows top to bottom: hello, idle, thinking, working, waiting_approval, needs_input, done, error; columns are frames."
        )
    }

    // MARK: - hatch_pet

    private static func hatchPet(arguments: [String: Any]) -> ToolResult {
        guard let rawDefinition = arguments["definition"] as? [String: Any] else {
            return .failure("`definition` must be the full pet JSON object (not a string).")
        }

        let definition: PetDefinition
        do {
            let data = try JSONSerialization.data(withJSONObject: rawDefinition)
            definition = try PetDefinition.decode(data)
        } catch let decodingError as DecodingError {
            return .failure("Invalid pet JSON structure: \(CommandLineInterface.describeDecodingError(decodingError))")
        } catch {
            return .failure("Invalid pet JSON: \(error.localizedDescription)")
        }

        let warnings: [String]
        do {
            warnings = try definition.validate()
        } catch {
            return .failure("""
            Validation failed:
            \(error.localizedDescription)

            Usual culprit: rows with mismatched widths — count the characters of every row; all \
            frames of all states must share one grid size. Fix the definition and call hatch_pet again.
            """)
        }

        let fileURL = AppPaths.petsDirectory.appendingPathComponent(definition.id + ".json")
        let replacedExisting = FileManager.default.fileExists(atPath: fileURL.path)
        do {
            try AppPaths.ensureDirectoryExists(AppPaths.petsDirectory)
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            try encoder.encode(definition).write(to: fileURL, options: .atomic)
        } catch {
            return .failure("Could not save \(fileURL.path): \(error.localizedDescription)")
        }

        var lines: [String] = []
        lines.append("Hatched \(definition.name) the \(definition.species) → \(fileURL.path)\(replacedExisting ? " (replaced the previous version)" : "")")
        if BuiltInPets.loadAll().contains(where: { $0.id == definition.id }) {
            lines.append("Note: this id shadows the built-in \"\(definition.id)\" until the file is deleted.")
        }
        for warning in warnings {
            lines.append("warning: \(warning)")
        }
        lines.append("Pick it via the menu bar 🐾 → Pet → \(definition.name) (\"Reload pets\" first if the overlay is already running).")
        lines.append("Check the sheet below — silhouette readable? eyes visible? states distinct? Iterate with hatch_pet if not.")
        return sheetResult(for: definition, text: lines.joined(separator: "\n"))
    }

    // MARK: - Sheet rendering

    /// Renders the contact sheet to a temporary directory and returns it inline as image
    /// content (plus `text`). Falls back to ASCII if PNG rendering fails.
    private static func sheetResult(for definition: PetDefinition, text: String) -> ToolResult {
        let temporaryDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("claude-airou-mcp-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: temporaryDirectory) }
        do {
            try SpriteRenderer.renderAll(pet: definition, outputDirectory: temporaryDirectory, pixelScale: 8, backgroundColor: nil)
            let sheetData = try Data(contentsOf: temporaryDirectory.appendingPathComponent("sheet.png"))
            return ToolResult(content: [
                ["type": "text", "text": text],
                ["type": "image", "data": sheetData.base64EncodedString(), "mimeType": "image/png"],
            ])
        } catch {
            let ascii = definition.frames(for: .idle).first.map { SpriteRenderer.asciiArt(frame: $0, solid: false) } ?? ""
            return .text("\(text)\n(rendering the sheet failed: \(error.localizedDescription))\nASCII idle frame:\n\(ascii)")
        }
    }
}
