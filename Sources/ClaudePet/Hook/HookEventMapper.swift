import Foundation

/// The subset of the Claude Code hook stdin JSON that the pet cares about.
/// Field names follow https://code.claude.com/docs/en/hooks
struct HookInput {
    let rawObject: [String: Any]

    init(rawObject: [String: Any]) {
        self.rawObject = rawObject
    }

    static func parse(data: Data) -> HookInput? {
        guard let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return nil
        }
        return HookInput(rawObject: object)
    }

    private func string(_ key: String) -> String? {
        rawObject[key] as? String
    }

    var sessionId: String { string("session_id") ?? "unknown-session" }
    var cwd: String { string("cwd") ?? FileManager.default.currentDirectoryPath }
    var hookEventName: String { string("hook_event_name") ?? "" }
    var toolName: String? { string("tool_name") }
    var toolInput: [String: Any] { rawObject["tool_input"] as? [String: Any] ?? [:] }
    var toolUseId: String? { string("tool_use_id") }
    var notificationType: String? { string("notification_type") }
    /// `message` is used by Notification and Elicitation events.
    var notificationMessage: String? { string("message") }
    var errorText: String? { string("error") }
    var errorType: String? { string("error_type") }
    var sessionStartSource: String? { string("source") }
    var sessionEndReason: String? { string("reason") }
    var agentType: String? { string("agent_type") }
    /// Present when the event was fired from inside a subagent (settings hooks also run there).
    var agentId: String? { string("agent_id") }
    var compactTrigger: String? { string("trigger") }
    var mcpServerName: String? { string("mcp_server_name") }
    /// ElicitationResult: "accept" | "decline" | "cancel"
    var elicitationAction: String? { string("action") }

    var isSubagentEvent: Bool { agentId != nil }
}

enum HookMappingResult: Equatable {
    case update(state: PetState, message: String, toolName: String?)
    case removeSession
    case ignore
}

/// Pure mapping from a hook event to what the pet should do. No I/O here so it is testable.
enum HookEventMapper {
    /// Events the installer registers. Anything else is ignored if it ever arrives.
    static let subscribedEventNames: [String] = [
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
    ]

    static func map(_ input: HookInput) -> HookMappingResult {
        switch input.hookEventName {
        case "SessionStart":
            switch input.sessionStartSource {
            case "compact":
                // Fires mid-turn after compaction; Claude keeps working, so don't greet.
                return .update(state: .thinking, message: "Context compacted, back to work", toolName: nil)
            case "resume":
                return .update(state: .hello, message: "Welcome back!", toolName: nil)
            case "clear":
                return .update(state: .hello, message: "Fresh start!", toolName: nil)
            default:
                return .update(state: .hello, message: "Hi! Ready when you are", toolName: nil)
            }

        case "SessionEnd":
            return .removeSession

        case "UserPromptSubmit":
            return .update(state: .thinking, message: "Thinking…", toolName: nil)

        case "PreToolUse":
            if let interaction = mapUserInteractionTool(input) { return interaction }
            let summary = ToolSummarizer.summarize(toolName: input.toolName ?? "tool", toolInput: input.toolInput)
            return .update(state: .working, message: summary, toolName: input.toolName)

        case "PostToolUse", "PostToolBatch":
            return .update(state: .thinking, message: "Thinking…", toolName: input.toolName)

        case "PostToolUseFailure":
            let name = input.toolName ?? "tool"
            return .update(state: .error, message: "\(name) failed — recovering…", toolName: input.toolName)

        case "PermissionRequest":
            if let interaction = mapUserInteractionTool(input) { return interaction }
            let summary = ToolSummarizer.summarize(toolName: input.toolName ?? "tool", toolInput: input.toolInput)
            return .update(state: .waitingApproval, message: "Approve? \(summary)", toolName: input.toolName)

        case "Notification":
            return mapNotification(input)

        case "Stop":
            return .update(state: .done, message: "Done!", toolName: nil)

        case "StopFailure":
            let detail = input.errorType.map { $0.replacingOccurrences(of: "_", with: " ") } ?? "API error"
            return .update(state: .error, message: "Stopped: \(detail)", toolName: nil)

        case "SubagentStart":
            let agent = input.agentType ?? "sub"
            return .update(state: .working, message: "Sent a \(agent) agent to work", toolName: "Agent")

        case "SubagentStop":
            return .update(state: .thinking, message: "Agent reported back", toolName: "Agent")

        case "PreCompact":
            let trigger = input.compactTrigger == "auto" ? "Auto-compacting" : "Compacting"
            return .update(state: .thinking, message: "\(trigger) context…", toolName: nil)

        case "PostCompact":
            return .update(state: .thinking, message: "Context compacted", toolName: nil)

        case "Elicitation":
            // Elicitation input carries mcp_server_name + message (no tool_name).
            let text = input.notificationMessage?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            let server = input.mcpServerName.map { " (\($0))" } ?? ""
            let message = text.isEmpty ? "A tool needs your input\(server)" : ToolSummarizer.truncate(text)
            return .update(state: .needsInput, message: message, toolName: input.mcpServerName)

        case "ElicitationResult":
            switch input.elicitationAction {
            case "decline", "cancel":
                return .update(state: .thinking, message: "Okay, skipping that", toolName: input.mcpServerName)
            default:
                return .update(state: .working, message: "Thanks! Continuing…", toolName: input.mcpServerName)
            }

        default:
            return .ignore
        }
    }

    /// Tools that block on the user are "needs input", not "working"/"approve?".
    private static func mapUserInteractionTool(_ input: HookInput) -> HookMappingResult? {
        switch input.toolName {
        case "AskUserQuestion":
            return .update(state: .needsInput, message: "Asking you a question", toolName: input.toolName)
        case "ExitPlanMode":
            return .update(state: .needsInput, message: "Waiting for plan approval", toolName: input.toolName)
        default:
            return nil
        }
    }

    private static func mapNotification(_ input: HookInput) -> HookMappingResult {
        let text = input.notificationMessage?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        switch input.notificationType {
        case "permission_prompt":
            return .update(state: .waitingApproval, message: text.isEmpty ? "Needs your approval" : ToolSummarizer.truncate(text), toolName: input.toolName)
        case "idle_prompt":
            // "Claude finished ~60 s ago and you haven't typed": the session is simply idle at the
            // prompt. Writing idle also clears a stuck busy state after an interrupt (Stop does not fire then).
            return .update(state: .idle, message: "", toolName: nil)
        case "agent_needs_input":
            return .update(state: .needsInput, message: text.isEmpty ? "Needs your input" : ToolSummarizer.truncate(text), toolName: nil)
        case "elicitation_dialog", "elicitation_url_dialog":
            return .update(state: .needsInput, message: text.isEmpty ? "A tool needs your input" : ToolSummarizer.truncate(text), toolName: nil)
        case "agent_completed":
            return .update(state: .done, message: text.isEmpty ? "Done!" : ToolSummarizer.truncate(text), toolName: nil)
        case "auth_success", "elicitation_complete", "elicitation_response":
            return .ignore
        default:
            return .ignore
        }
    }
}

/// Turns a tool call into a short, human-readable speech bubble line.
enum ToolSummarizer {
    private static let maxCharacters = 48

    static func summarize(toolName: String, toolInput: [String: Any]) -> String {
        func lastPathComponent(_ key: String) -> String? {
            guard let path = toolInput[key] as? String, !path.isEmpty else { return nil }
            return URL(fileURLWithPath: path).lastPathComponent
        }
        func value(_ key: String) -> String? {
            guard let text = toolInput[key] as? String else { return nil }
            let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
            return trimmed.isEmpty ? nil : trimmed
        }

        let line: String
        switch toolName {
        case "Read":
            line = "Reading \(lastPathComponent("file_path") ?? "a file")"
        case "Edit", "MultiEdit":
            line = "Editing \(lastPathComponent("file_path") ?? "a file")"
        case "Write":
            line = "Writing \(lastPathComponent("file_path") ?? "a file")"
        case "NotebookEdit":
            line = "Editing \(lastPathComponent("notebook_path") ?? "a notebook")"
        case "Bash":
            let description = value("description")
            let command = value("command")?.replacingOccurrences(of: "\n", with: " ")
            line = "Running: \(description ?? command ?? "a command")"
        case "Grep":
            line = "Searching for “\(value("pattern") ?? "…")”"
        case "Glob":
            line = "Looking for \(value("pattern") ?? "files")"
        case "WebFetch":
            if let urlText = value("url"), let host = URL(string: urlText)?.host {
                line = "Fetching \(host)"
            } else {
                line = "Fetching a page"
            }
        case "WebSearch":
            line = "Searching the web: \(value("query") ?? "…")"
        case "Agent", "Task":
            line = "Delegating: \(value("description") ?? "a subtask")"
        case "TodoWrite", "TaskCreate", "TaskUpdate":
            line = "Updating the task list"
        case "AskUserQuestion":
            line = "Asking you a question"
        case "ExitPlanMode":
            line = "Waiting for plan approval"
        case "Skill":
            line = "Using skill \(value("skill") ?? "")"
        default:
            if toolName.hasPrefix("mcp__") {
                let parts = toolName.split(separator: "_", omittingEmptySubsequences: true)
                let readable = parts.dropFirst().joined(separator: " ")
                line = "Using \(readable.isEmpty ? "an MCP tool" : readable)"
            } else {
                line = "Using \(toolName)"
            }
        }
        return truncate(line)
    }

    static func truncate(_ text: String) -> String {
        if text.count <= maxCharacters { return text }
        let endIndex = text.index(text.startIndex, offsetBy: maxCharacters - 1)
        return String(text[..<endIndex]) + "…"
    }
}
