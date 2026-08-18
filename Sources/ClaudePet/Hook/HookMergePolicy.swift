import Foundation

/// Decides whether a freshly mapped event may overwrite the session's current snapshot.
///
/// Claude Code fires hooks concurrently for parallel tool calls and also from inside subagents
/// (same `session_id`, with `agent_id`). A naive last-writer-wins would let a sibling tool's
/// PostToolUse wipe a pending "waiting for approval" — the one state the user must not miss.
enum HookMergePolicy {
    enum Resolution: Equatable {
        case write(SessionSnapshot)
        case keep(reason: String)
    }

    static func resolve(
        existing: SessionSnapshot?,
        input: HookInput,
        mappedState: PetState,
        message: String,
        toolName: String?,
        now: Date = Date()
    ) -> Resolution {
        let existingState = existing?.effectiveState ?? .idle
        let userIsBlocked = existingState.isAttentionNeeded

        if userIsBlocked, let existing {
            // Subagents keep running while the main thread waits on the user; ignore their chatter.
            if input.isSubagentEvent {
                return .keep(reason: "subagent \(input.hookEventName) while \(existing.state.rawValue)")
            }
            // A sibling tool from the same batch finished — the awaited call is still pending.
            let isToolCompletion = input.hookEventName == "PostToolUse" || input.hookEventName == "PostToolUseFailure"
            if isToolCompletion,
               let pending = existing.pendingToolUseId,
               let finished = input.toolUseId,
               pending != finished {
                return .keep(reason: "sibling tool \(finished) finished while waiting on \(pending)")
            }
        }

        var snapshot = SessionSnapshot(
            sessionId: input.sessionId,
            cwd: input.cwd,
            state: mappedState,
            message: message,
            lastEventName: input.hookEventName,
            toolName: toolName,
            updatedAtEpochSeconds: now.timeIntervalSince1970
        )
        if mappedState.isAttentionNeeded {
            // PermissionRequest / PreToolUse(AskUserQuestion) carry tool_use_id; a Notification
            // re-asserting the same wait does not, so inherit the id we already had.
            snapshot.pendingToolUseId = input.toolUseId ?? (userIsBlocked ? existing?.pendingToolUseId : nil)
        }
        return .write(snapshot)
    }
}
