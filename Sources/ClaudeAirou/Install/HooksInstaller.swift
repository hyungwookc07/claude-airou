import Foundation

/// Adds/removes the `claude-airou hook` entries in `~/.claude/settings.json`.
///
/// The merge is conservative: only the `hooks` object is touched, existing entries for other
/// tools are left alone, a timestamped backup is written first, and the operation is idempotent
/// (re-running updates the command path instead of duplicating entries).
struct HooksInstaller {
    struct Report {
        var settingsPath: String
        var backupPath: String?
        var addedEvents: [String] = []
        var updatedEvents: [String] = []
        var removedEvents: [String] = []
        var unchangedEvents: [String] = []
        var hookCommand: String

        var summaryText: String {
            var lines: [String] = []
            lines.append("Settings: \(settingsPath)")
            if let backupPath { lines.append("Backup:   \(backupPath)") }
            lines.append("Command:  \(hookCommand)")
            if !addedEvents.isEmpty { lines.append("Added:    \(addedEvents.joined(separator: ", "))") }
            if !updatedEvents.isEmpty { lines.append("Updated:  \(updatedEvents.joined(separator: ", "))") }
            if !removedEvents.isEmpty { lines.append("Removed:  \(removedEvents.joined(separator: ", "))") }
            if !unchangedEvents.isEmpty { lines.append("Unchanged: \(unchangedEvents.count) event(s)") }
            return lines.joined(separator: "\n")
        }
    }

    struct InstallError: LocalizedError {
        let message: String
        var errorDescription: String? { message }
    }

    /// Marker used to recognise our own entries regardless of where the binary lives.
    static let commandMarker = "claude-airou"
    /// Entries written before the rename; recognised so install updates them and uninstall removes them.
    static let legacyCommandMarkers = ["claude-pet"]

    static func containsOurMarker(_ text: String) -> Bool {
        text.contains(commandMarker) || legacyCommandMarkers.contains { text.contains($0) }
    }
    static let hookSubcommand = "hook"
    static let hookTimeoutSeconds = 10

    let settingsURL: URL
    let executablePath: String

    init(settingsURL: URL = AppPaths.claudeSettingsFile, executablePath: String? = nil) {
        self.settingsURL = settingsURL
        self.executablePath = executablePath ?? Self.currentExecutablePath()
    }

    /// The shell command Claude Code will run (`sh -c`). Single-quoted so spaces, `$`, backticks
    /// and double quotes in the path are all literal.
    var hookCommand: String {
        "\(Self.shellSingleQuoted(executablePath)) \(Self.hookSubcommand)"
    }

    static func shellSingleQuoted(_ text: String) -> String {
        "'" + text.replacingOccurrences(of: "'", with: "'\\''") + "'"
    }

    /// Absolute path of this binary as invoked. Symlinks are kept on purpose: if the user
    /// installed `~/.local/bin/claude-airou -> <build dir>`, the symlink is the stable address.
    static func currentExecutablePath() -> String {
        let rawPath = Bundle.main.executablePath ?? CommandLine.arguments[0]
        return URL(fileURLWithPath: rawPath).standardizedFileURL.path
    }

    // MARK: - Install

    func install() throws -> Report {
        var settings = try loadSettings()
        var hooks = try hooksObject(from: settings)
        var report = Report(settingsPath: settingsURL.path, hookCommand: hookCommand)

        for eventName in HookEventMapper.subscribedEventNames {
            var groups = try hookGroups(from: hooks, eventName: eventName)
            var foundOurs = false
            var changed = false

            for groupIndex in groups.indices {
                var group = groups[groupIndex]
                var handlers = try handlers(from: group, eventName: eventName, groupIndex: groupIndex)
                for handlerIndex in handlers.indices where Self.isOurHandler(handlers[handlerIndex]) {
                    foundOurs = true
                    let desired = Self.ourHandler(command: hookCommand)
                    if !NSDictionary(dictionary: handlers[handlerIndex]).isEqual(to: desired) {
                        handlers[handlerIndex] = desired
                        changed = true
                    }
                }
                group["hooks"] = handlers
                groups[groupIndex] = group
            }

            if !foundOurs {
                groups.append(["hooks": [Self.ourHandler(command: hookCommand)]])
                report.addedEvents.append(eventName)
            } else if changed {
                report.updatedEvents.append(eventName)
            } else {
                report.unchangedEvents.append(eventName)
            }
            hooks[eventName] = groups
        }

        // Nothing to do: leave the file byte-for-byte alone (no backup, no reformatting).
        if report.addedEvents.isEmpty && report.updatedEvents.isEmpty {
            return report
        }

        settings["hooks"] = hooks
        report.backupPath = try backupIfNeeded()
        try writeSettings(settings)
        return report
    }

    // MARK: - Uninstall

    func uninstall() throws -> Report {
        var settings = try loadSettings()
        var report = Report(settingsPath: settingsURL.path, hookCommand: hookCommand)
        guard settings["hooks"] != nil else { return report }
        var hooks = try hooksObject(from: settings)

        for eventName in hooks.keys.sorted() {
            guard var groups = hooks[eventName] as? [[String: Any]] else { continue } // foreign shape: leave alone
            var removedAny = false
            groups = groups.compactMap { group -> [String: Any]? in
                var group = group
                guard let handlers = group["hooks"] as? [[String: Any]] else { return group } // foreign shape: leave alone
                let kept = handlers.filter { !Self.isOurHandler($0) }
                if kept.count != handlers.count { removedAny = true }
                if kept.isEmpty && !handlers.isEmpty { return nil } // group only held our handler
                group["hooks"] = kept
                return group
            }
            if removedAny { report.removedEvents.append(eventName) }
            if groups.isEmpty {
                hooks.removeValue(forKey: eventName)
            } else {
                hooks[eventName] = groups
            }
        }

        if report.removedEvents.isEmpty {
            return report
        }

        if hooks.isEmpty {
            settings.removeValue(forKey: "hooks")
        } else {
            settings["hooks"] = hooks
        }
        report.backupPath = try backupIfNeeded()
        try writeSettings(settings)
        return report
    }

    // MARK: - Helpers

    static func ourHandler(command: String) -> [String: Any] {
        [
            "type": "command",
            "command": command,
            "timeout": hookTimeoutSeconds,
        ]
    }

    /// Recognises our own entries regardless of where the binary lives:
    /// shell form `'.../claude-airou' hook` (what we write) or exec form `command: ".../claude-airou", args: ["hook"]`.
    static func isOurHandler(_ handler: [String: Any]) -> Bool {
        guard handler["type"] as? String == "command",
              let command = handler["command"] as? String else { return false }
        let trimmed = command.trimmingCharacters(in: .whitespaces)
        guard containsOurMarker(trimmed) else { return false }
        if trimmed.hasSuffix(" " + hookSubcommand) { return true }
        if let args = handler["args"] as? [String], args == [hookSubcommand] {
            let markers = [commandMarker] + legacyCommandMarkers
            return markers.contains { trimmed.hasSuffix($0) || trimmed.hasSuffix($0 + "'") || trimmed.hasSuffix($0 + "\"") }
        }
        return false
    }

    /// `settings.hooks` must be an object if present. Anything else is refused rather than clobbered.
    private func hooksObject(from settings: [String: Any]) throws -> [String: Any] {
        guard let raw = settings["hooks"] else { return [:] }
        guard let object = raw as? [String: Any] else {
            throw InstallError(message: "\"hooks\" in \(settingsURL.path) is not a JSON object; fix or remove it and re-run.")
        }
        return object
    }

    private func hookGroups(from hooks: [String: Any], eventName: String) throws -> [[String: Any]] {
        guard let raw = hooks[eventName] else { return [] }
        guard let groups = raw as? [[String: Any]] else {
            throw InstallError(message: "hooks.\(eventName) in \(settingsURL.path) is not an array of hook groups; fix or remove it and re-run.")
        }
        return groups
    }

    private func handlers(from group: [String: Any], eventName: String, groupIndex: Int) throws -> [[String: Any]] {
        guard let raw = group["hooks"] else { return [] }
        guard let handlers = raw as? [[String: Any]] else {
            throw InstallError(message: "hooks.\(eventName)[\(groupIndex)].hooks in \(settingsURL.path) is not an array; fix or remove it and re-run.")
        }
        return handlers
    }

    private func loadSettings() throws -> [String: Any] {
        guard FileManager.default.fileExists(atPath: settingsURL.path) else { return [:] }
        let data = try Data(contentsOf: settingsURL)
        if data.isEmpty { return [:] }
        guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw InstallError(message: "\(settingsURL.path) is not a JSON object; refusing to modify it.")
        }
        return object
    }

    private func backupIfNeeded() throws -> String? {
        guard FileManager.default.fileExists(atPath: settingsURL.path) else { return nil }
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyyMMdd-HHmmss"
        let baseName = "\(settingsURL.lastPathComponent).claude-airou-backup-\(formatter.string(from: Date()))"
        let directory = settingsURL.deletingLastPathComponent()
        var backupURL = directory.appendingPathComponent(baseName)
        var suffix = 1
        while FileManager.default.fileExists(atPath: backupURL.path) {
            backupURL = directory.appendingPathComponent("\(baseName)-\(suffix)")
            suffix += 1
        }
        try FileManager.default.copyItem(at: settingsURL, to: backupURL)
        return backupURL.path
    }

    private func writeSettings(_ settings: [String: Any]) throws {
        try AppPaths.ensureDirectoryExists(settingsURL.deletingLastPathComponent())
        let data = try JSONSerialization.data(
            withJSONObject: settings,
            options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
        )
        try (data + Data("\n".utf8)).write(to: settingsURL, options: .atomic)
    }

    /// The JSON a user would paste by hand (printed by `claude-airou install-hooks --print`).
    func snippetJSON() -> String {
        var hooks: [String: Any] = [:]
        for eventName in HookEventMapper.subscribedEventNames {
            hooks[eventName] = [["hooks": [Self.ourHandler(command: hookCommand)]]]
        }
        let data = (try? JSONSerialization.data(
            withJSONObject: ["hooks": hooks],
            options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
        )) ?? Data()
        return String(decoding: data, as: UTF8.self)
    }
}
