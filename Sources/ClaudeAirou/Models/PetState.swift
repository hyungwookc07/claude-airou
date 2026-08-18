import Foundation

/// The mood/state a pet can display. Mirrors what OpenAI Codex pets show:
/// thinking, working, waiting for approval, needing input, done, error.
enum PetState: String, Codable, CaseIterable {
    case hello
    case idle
    case thinking
    case working
    case waitingApproval = "waiting_approval"
    case needsInput = "needs_input"
    case done
    case error

    var isBusy: Bool { self == .thinking || self == .working }

    var isAttentionNeeded: Bool { self == .waitingApproval || self == .needsInput }

    /// Seconds after which the state decays back to `idle` if no newer event arrives.
    /// `nil` means the state is sticky.
    ///
    /// The long ones are safety nets only: Claude Code has no hook for "user denied the
    /// permission" or "user pressed Esc" (Stop does not fire on interrupts), so without a
    /// decay a session could show the red clock forever. Real transitions come from
    /// UserPromptSubmit / PreToolUse / PostToolUse / PostToolBatch / Stop / idle_prompt.
    var transientDurationSeconds: TimeInterval? {
        switch self {
        case .hello: return 4
        case .done: return 6
        case .error: return 8
        case .waitingApproval, .needsInput: return 20 * 60
        case .thinking, .working: return 15 * 60
        case .idle: return nil
        }
    }

    /// Fallback chain used when a pet JSON lacks frames for a state.
    var fallbackStates: [PetState] {
        switch self {
        case .hello: return [.done, .idle]
        case .working: return [.thinking, .idle]
        case .waitingApproval: return [.needsInput, .idle]
        case .needsInput: return [.waitingApproval, .idle]
        case .thinking, .done, .error: return [.idle]
        case .idle: return []
        }
    }

    var displayLabel: String {
        switch self {
        case .hello: return "Hello"
        case .idle: return "Idle"
        case .thinking: return "Thinking"
        case .working: return "Working"
        case .waitingApproval: return "Waiting for approval"
        case .needsInput: return "Needs your input"
        case .done: return "Done"
        case .error: return "Error"
        }
    }

    /// Accepts both `waiting_approval` and `waitingApproval` spellings (CLI convenience).
    static func parse(_ text: String) -> PetState? {
        let normalized = text
            .replacingOccurrences(of: "-", with: "_")
            .lowercased()
        if let direct = PetState(rawValue: normalized) { return direct }
        switch normalized {
        case "waitingapproval", "waiting", "approval", "permission": return .waitingApproval
        case "needsinput", "input", "question": return .needsInput
        case "ok", "success", "complete", "completed", "finished": return .done
        case "fail", "failed", "failure": return .error
        case "busy": return .working
        default: return nil
        }
    }
}

/// One Claude Code session as last reported by the hook.
struct SessionSnapshot: Codable, Equatable {
    var sessionId: String
    var cwd: String
    var state: PetState
    var message: String
    var lastEventName: String
    var toolName: String?
    var updatedAtEpochSeconds: Double
    /// While waiting on the user for a specific tool call (permission / question), the id of that
    /// call. Sibling tool calls finishing in the same batch must not clear the wait.
    var pendingToolUseId: String?

    var projectName: String {
        let name = URL(fileURLWithPath: cwd).lastPathComponent
        return name.isEmpty ? cwd : name
    }

    var updatedAt: Date { Date(timeIntervalSince1970: updatedAtEpochSeconds) }

    var ageSeconds: TimeInterval { Date().timeIntervalSince(updatedAt) }

    /// The state to actually show right now, after transient decay.
    var effectiveState: PetState {
        if let duration = state.transientDurationSeconds, ageSeconds > duration {
            return .idle
        }
        return state
    }
}
