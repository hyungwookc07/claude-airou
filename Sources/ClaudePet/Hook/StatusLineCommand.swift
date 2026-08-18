import Foundation

/// `claude-pet statusline` — installed as the Claude Code `statusLine` command. Records the usage
/// figures Claude Code hands to the status line (context window, rate limits, cost) for the pet's
/// battery gauge, then runs whatever status line the user had before, with the same stdin, so the
/// terminal status line looks exactly as it did.
enum StatusLineCommand {
    /// The user's original `statusLine` object from settings.json, kept here while ours is installed.
    static var passthroughFile: URL { AppPaths.rootDirectory.appendingPathComponent("statusline-passthrough.json") }

    static func run(arguments: [String], stateStore: SessionStateStore = SessionStateStore()) -> Int32 {
        if isatty(FileHandle.standardInput.fileDescriptor) != 0 {
            StandardError.print("claude-pet statusline: expects the Claude Code status line JSON on stdin (see `claude-pet install-statusline`).")
            return 0
        }
        let inputData = FileHandle.standardInput.readDataToEndOfFile()

        if let object = try? JSONSerialization.jsonObject(with: inputData) as? [String: Any],
           let usage = parseUsage(object) {
            try? stateStore.writeUsage(usage)
        }

        return runPassthrough(inputData: inputData, explicitCommand: explicitThenCommand(arguments))
    }

    /// `--then CMD` overrides the stored passthrough (handy for testing).
    private static func explicitThenCommand(_ arguments: [String]) -> String? {
        guard let index = arguments.firstIndex(of: "--then"), index + 1 < arguments.count else { return nil }
        return arguments[index + 1]
    }

    // MARK: - Parsing

    static func parseUsage(_ object: [String: Any], now: Date = Date()) -> SessionUsageSnapshot? {
        guard let sessionId = object["session_id"] as? String, !sessionId.isEmpty else { return nil }
        func number(_ container: [String: Any]?, _ key: String) -> Double? {
            guard let value = container?[key] else { return nil }
            if let double = value as? Double { return double }
            if let int = value as? Int { return Double(int) }
            if let text = value as? String { return Double(text) }
            return nil
        }
        func date(_ container: [String: Any]?, _ key: String) -> Double? {
            guard let value = container?[key] else { return nil }
            if let seconds = value as? Double { return seconds > 1e12 ? seconds / 1000 : seconds } // ms vs s
            if let seconds = value as? Int { return Double(seconds) > 1e12 ? Double(seconds) / 1000 : Double(seconds) }
            if let text = value as? String {
                if let seconds = Double(text) { return seconds > 1e12 ? seconds / 1000 : seconds }
                let formatter = ISO8601DateFormatter()
                formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
                if let parsed = formatter.date(from: text) { return parsed.timeIntervalSince1970 }
                formatter.formatOptions = [.withInternetDateTime]
                if let parsed = formatter.date(from: text) { return parsed.timeIntervalSince1970 }
            }
            return nil
        }

        let contextWindow = object["context_window"] as? [String: Any]
        let rateLimits = object["rate_limits"] as? [String: Any]
        let fiveHour = rateLimits?["five_hour"] as? [String: Any]
        let sevenDay = rateLimits?["seven_day"] as? [String: Any]
        let cost = object["cost"] as? [String: Any]
        let model = object["model"] as? [String: Any]

        var contextUsed = number(contextWindow, "used_percentage")
        let contextSize = number(contextWindow, "context_window_size").map { Int($0) }
        var contextTokens: Int?
        if let currentUsage = contextWindow?["current_usage"] as? [String: Any] {
            let input = number(currentUsage, "input_tokens") ?? 0
            let cacheCreation = number(currentUsage, "cache_creation_input_tokens") ?? 0
            let cacheRead = number(currentUsage, "cache_read_input_tokens") ?? 0
            let total = input + cacheCreation + cacheRead
            if total > 0 { contextTokens = Int(total) }
            if contextUsed == nil, let contextSize, contextSize > 0, total > 0 {
                contextUsed = total / Double(contextSize) * 100
            }
        }

        var usage = SessionUsageSnapshot(sessionId: sessionId, source: .statusLine, updatedAtEpochSeconds: now.timeIntervalSince1970)
        usage.contextUsedPercentage = contextUsed
        usage.contextWindowSize = contextSize
        usage.contextTokens = contextTokens
        usage.totalInputTokens = number(contextWindow, "total_input_tokens").map { Int($0) }
        usage.totalOutputTokens = number(contextWindow, "total_output_tokens").map { Int($0) }
        usage.modelDisplayName = model?["display_name"] as? String
        usage.fiveHourUsedPercentage = number(fiveHour, "used_percentage")
        usage.fiveHourResetsAtEpochSeconds = date(fiveHour, "resets_at")
        usage.sevenDayUsedPercentage = number(sevenDay, "used_percentage")
        usage.sevenDayResetsAtEpochSeconds = date(sevenDay, "resets_at")
        usage.totalCostUSD = number(cost, "total_cost_usd")
        return usage
    }

    // MARK: - Passthrough

    /// Runs the user's own status line command with the same stdin and forwards its output; returns its exit code.
    private static func runPassthrough(inputData: Data, explicitCommand: String?) -> Int32 {
        let command = explicitCommand ?? storedPassthroughCommand()
        guard let command, !command.trimmingCharacters(in: .whitespaces).isEmpty else {
            return 0 // no original status line: print nothing
        }
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/sh")
        process.arguments = ["-c", command]
        let stdinPipe = Pipe()
        process.standardInput = stdinPipe
        process.standardOutput = FileHandle.standardOutput
        process.standardError = FileHandle.standardError
        do {
            try process.run()
        } catch {
            StandardError.print("claude-pet statusline: could not run passthrough: \(error.localizedDescription)")
            return 0
        }
        stdinPipe.fileHandleForWriting.write(inputData)
        try? stdinPipe.fileHandleForWriting.close()
        process.waitUntilExit()
        return process.terminationStatus
    }

    static func storedPassthroughCommand() -> String? {
        guard let object = storedPassthroughObject() else { return nil }
        guard (object["type"] as? String ?? "command") == "command" else { return nil }
        return object["command"] as? String
    }

    static func storedPassthroughObject() -> [String: Any]? {
        guard let data = try? Data(contentsOf: passthroughFile),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }
        return object
    }
}
