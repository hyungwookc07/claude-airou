import Foundation

/// Entry point for `claude-pet hook`, registered in `~/.claude/settings.json` for every event.
///
/// Contract with Claude Code:
///  - never write to stdout (UserPromptSubmit / SessionStart stdout is injected into the model context)
///  - always exit 0 (a non-zero exit would surface as a hook error inside Claude Code)
///  - be fast (this runs synchronously before/after every tool call)
enum HookCommand {
    static let hookLogMaxBytes: UInt64 = 512 * 1024

    static func run(stateStore: SessionStateStore = SessionStateStore()) -> Int32 {
        if isatty(FileHandle.standardInput.fileDescriptor) != 0 {
            StandardError.print("claude-pet hook: expects Claude Code hook JSON on stdin (see `claude-pet install-hooks`).")
            return 0
        }

        let inputData = FileHandle.standardInput.readDataToEndOfFile()
        guard let input = HookInput.parse(data: inputData) else {
            appendLog("unparseable input (\(inputData.count) bytes)")
            return 0
        }

        let mapping = HookEventMapper.map(input)
        switch mapping {
        case .ignore:
            appendLog("\(input.hookEventName) \(input.sessionId) ignored")

        case .removeSession:
            stateStore.remove(sessionId: input.sessionId)
            appendLog("\(input.hookEventName) \(input.sessionId) removed")

        case let .update(state, message, toolName):
            let existing = stateStore.read(sessionId: input.sessionId)
            let resolution = HookMergePolicy.resolve(
                existing: existing,
                input: input,
                mappedState: state,
                message: message,
                toolName: toolName
            )
            switch resolution {
            case let .keep(reason):
                appendLog("\(input.hookEventName) \(input.sessionId) kept (\(reason))")
            case let .write(snapshot):
                do {
                    try stateStore.write(snapshot)
                    let agentSuffix = input.agentId.map { " [agent \($0.prefix(8))]" } ?? ""
                    appendLog("\(input.hookEventName)\(agentSuffix) \(input.sessionId) -> \(state.rawValue) \"\(message)\"")
                } catch {
                    appendLog("\(input.hookEventName) \(input.sessionId) write failed: \(error.localizedDescription)")
                }
            }
        }
        return 0
    }

    // MARK: - Logging

    private static let logTimestampFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyy-MM-dd HH:mm:ss"
        return formatter
    }()

    /// Appends one line to `~/.claude-pet/hook.log`, truncating the file when it grows past `hookLogMaxBytes`.
    static func appendLog(_ line: String) {
        let logURL = AppPaths.hookLogFile
        do {
            try AppPaths.ensureDirectoryExists(logURL.deletingLastPathComponent())
            let fileManager = FileManager.default
            if let attributes = try? fileManager.attributesOfItem(atPath: logURL.path),
               let size = attributes[.size] as? UInt64, size > hookLogMaxBytes {
                try? fileManager.removeItem(at: logURL)
            }
            if !fileManager.fileExists(atPath: logURL.path) {
                fileManager.createFile(atPath: logURL.path, contents: nil)
            }
            let handle = try FileHandle(forWritingTo: logURL)
            defer { try? handle.close() }
            try handle.seekToEnd()
            let entry = "\(logTimestampFormatter.string(from: Date())) \(line)\n"
            try handle.write(contentsOf: Data(entry.utf8))
        } catch {
            // Logging is best-effort; never let it affect the hook outcome.
        }
    }
}
