import Foundation

/// `claude-airou statusline` — installed as the Claude Code `statusLine` command. Records the usage
/// figures Claude Code hands to the status line (context window, rate limits, cost) for the pet's
/// battery gauge, then runs whatever status line the user had before, with the same stdin, so the
/// terminal status line looks exactly as it did.
enum StatusLineCommand {
    /// Set in the passthrough's environment; if we ever see it on our own stdin path we are being
    /// invoked by ourselves and must not spawn again.
    static let recursionGuardEnvironmentKey = "CLAUDE_AIROU_STATUSLINE_DEPTH"

    /// The user's original `statusLine` object from settings.json, kept here while ours is installed.
    /// One file per settings file, so `--settings` targets don't clobber each other.
    static func passthroughFile(forSettingsPath settingsPath: String?) -> URL {
        let root = AppPaths.rootDirectory
        guard let settingsPath, settingsPath != AppPaths.claudeSettingsFile.path else {
            return root.appendingPathComponent("statusline-passthrough.json")
        }
        let digest = String(settingsPath.utf8.reduce(UInt64(5381)) { ($0 &* 33) &+ UInt64($1) }, radix: 36)
        return root.appendingPathComponent("statusline-passthrough-\(digest).json")
    }

    static func run(arguments: [String], stateStore: SessionStateStore = SessionStateStore()) -> Int32 {
        signal(SIGPIPE, SIG_IGN) // a passthrough that exits before draining stdin must not kill us

        if isatty(FileHandle.standardInput.fileDescriptor) != 0 {
            StandardError.print("claude-airou statusline: expects the Claude Code status line JSON on stdin (see `claude-airou install-statusline`).")
            return 0
        }
        let inputData = FileHandle.standardInput.readDataToEndOfFile()

        if let object = try? JSONSerialization.jsonObject(with: inputData) as? [String: Any],
           let usage = parseUsage(object) {
            try? stateStore.mergeUsage(usage)
        }

        let options = parseOptions(arguments)
        return runPassthrough(inputData: inputData, explicitCommand: options.thenCommand, settingsPath: options.settingsPath)
    }

    struct Options {
        var thenCommand: String?
        var settingsPath: String?
    }

    /// `--then CMD` / `--then=CMD` (testing) and `--settings PATH` / `--settings=PATH` (which passthrough file).
    static func parseOptions(_ arguments: [String]) -> Options {
        var options = Options()
        var index = 0
        while index < arguments.count {
            let argument = arguments[index]
            func value(after name: String) -> String? {
                if argument == name, index + 1 < arguments.count { index += 1; return arguments[index] }
                if argument.hasPrefix(name + "=") { return String(argument.dropFirst(name.count + 1)) }
                return nil
            }
            if let then = value(after: "--then") { options.thenCommand = then }
            else if let settings = value(after: "--settings") { options.settingsPath = (settings as NSString).expandingTildeInPath }
            index += 1
        }
        return options
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

    /// True when `command` is one of our own status line commands (would recurse forever).
    static func isSelfInvocation(_ command: String) -> Bool {
        let trimmed = command.trimmingCharacters(in: .whitespaces)
        guard HooksInstaller.containsOurMarker(trimmed) else { return false }
        return trimmed.hasSuffix(" statusline") || trimmed.contains(" statusline ")
    }

    /// Runs the user's own status line command with the same stdin and forwards its output; returns its exit code.
    private static func runPassthrough(inputData: Data, explicitCommand: String?, settingsPath: String?) -> Int32 {
        if ProcessInfo.processInfo.environment[recursionGuardEnvironmentKey] != nil {
            StandardError.print("claude-airou statusline: refusing to run nested (recursion guard).")
            return 0
        }
        let command = explicitCommand ?? storedPassthroughCommand(settingsPath: settingsPath)
        guard let command, !command.trimmingCharacters(in: .whitespaces).isEmpty else {
            return 0 // no original status line: print nothing
        }
        if isSelfInvocation(command) {
            StandardError.print("claude-airou statusline: stored passthrough is claude-airou itself; not running it. Run `claude-airou uninstall-statusline` then `install-statusline` to repair.")
            return 0
        }

        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/sh")
        process.arguments = ["-c", command]
        var environment = ProcessInfo.processInfo.environment
        environment[recursionGuardEnvironmentKey] = "1"
        process.environment = environment
        let stdinPipe = Pipe()
        process.standardInput = stdinPipe
        process.standardOutput = FileHandle.standardOutput
        process.standardError = FileHandle.standardError

        // Relay termination: if Claude Code cancels us, take the child down too.
        signal(SIGTERM, SIG_IGN)
        let terminationSource = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .global())
        terminationSource.setEventHandler {
            if process.isRunning { process.terminate() }
        }
        terminationSource.resume()
        defer { terminationSource.cancel() }

        do {
            try process.run()
        } catch {
            StandardError.print("claude-airou statusline: could not run passthrough: \(error.localizedDescription)")
            return 0
        }
        // The child may exit without reading stdin (SIGPIPE is ignored above; the write then just fails).
        try? stdinPipe.fileHandleForWriting.write(contentsOf: inputData)
        try? stdinPipe.fileHandleForWriting.close()
        process.waitUntilExit()
        return process.terminationStatus
    }

    static func storedPassthroughCommand(settingsPath: String?) -> String? {
        guard let object = storedPassthroughObject(settingsPath: settingsPath) else { return nil }
        guard (object["type"] as? String ?? "command") == "command" else { return nil }
        return object["command"] as? String
    }

    static func storedPassthroughObject(settingsPath: String?) -> [String: Any]? {
        let url = passthroughFile(forSettingsPath: settingsPath)
        guard let data = try? Data(contentsOf: url),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }
        return object
    }

    /// True when a passthrough file exists but cannot be read as a JSON object.
    static func passthroughFileIsCorrupt(settingsPath: String?) -> Bool {
        let url = passthroughFile(forSettingsPath: settingsPath)
        guard FileManager.default.fileExists(atPath: url.path) else { return false }
        return storedPassthroughObject(settingsPath: settingsPath) == nil
    }
}
