import Foundation

/// Swaps `settings.statusLine` for `claude-airou statusline`, keeping the user's original status line
/// object in `~/.claude-airou/statusline-passthrough*.json` so it keeps running (and can be restored).
struct StatusLineInstaller {
    struct Report {
        var settingsPath: String
        var backupPath: String?
        var action: String
        var passthroughCommand: String?

        var summaryText: String {
            var lines = ["Settings: \(settingsPath)"]
            if let backupPath { lines.append("Backup:   \(backupPath)") }
            lines.append("Action:   \(action)")
            if let passthroughCommand { lines.append("Then runs: \(passthroughCommand)") }
            return lines.joined(separator: "\n")
        }
    }

    static let subcommand = "statusline"
    /// Keys of the statusLine object that describe *how* to run us and are therefore replaced;
    /// everything else (padding, refreshInterval, hideVimModeIndicator, …) is carried over.
    static let replacedKeys: Set<String> = ["type", "command", "args"]

    let settingsURL: URL
    let executablePath: String

    init(settingsURL: URL = AppPaths.claudeSettingsFile, executablePath: String? = nil) {
        self.settingsURL = settingsURL
        self.executablePath = executablePath ?? HooksInstaller.currentExecutablePath()
    }

    private var isDefaultSettings: Bool { settingsURL.standardizedFileURL.path == AppPaths.claudeSettingsFile.standardizedFileURL.path }

    /// The passthrough file for this settings file (nil marker = default settings).
    private var passthroughSettingsPath: String? { isDefaultSettings ? nil : settingsURL.path }

    var ourCommand: String {
        var command = "\(HooksInstaller.shellSingleQuoted(executablePath)) \(Self.subcommand)"
        if !isDefaultSettings {
            command += " --settings \(HooksInstaller.shellSingleQuoted(settingsURL.path))"
        }
        return command
    }

    /// Our statusLine object, keeping any extra keys the user had (padding etc.).
    func ourStatusLineObject(preservingExtrasFrom original: [String: Any]?) -> [String: Any] {
        var object: [String: Any] = original?.filter { !Self.replacedKeys.contains($0.key) } ?? [:]
        object["type"] = "command"
        object["command"] = ourCommand
        return object
    }

    /// Recognises our own entry: a `... statusline` command whose text carries our marker (current or
    /// legacy name) or whose executable resolves to this very binary (renamed/symlinked installs).
    static func isOurs(_ statusLine: [String: Any], executablePath: String = HooksInstaller.currentExecutablePath()) -> Bool {
        guard (statusLine["type"] as? String ?? "command") == "command",
              let command = statusLine["command"] as? String else { return false }
        let trimmed = command.trimmingCharacters(in: .whitespaces)
        let words = trimmed.split(separator: " ", omittingEmptySubsequences: true).map(String.init)
        guard words.count >= 2, words[1...].contains(subcommand) else { return false }
        if HooksInstaller.containsOurMarker(trimmed) { return true }
        // Same file, different name: compare resolved executable paths.
        let firstWord = words[0].trimmingCharacters(in: CharacterSet(charactersIn: "'\""))
        let resolved = URL(fileURLWithPath: firstWord).resolvingSymlinksInPath().standardizedFileURL.path
        let ours = URL(fileURLWithPath: executablePath).resolvingSymlinksInPath().standardizedFileURL.path
        return resolved == ours
    }

    // MARK: - Install

    func install() throws -> Report {
        var settings = try loadSettings()
        var report = Report(settingsPath: settingsURL.path, action: "")

        if let existing = settings["statusLine"] {
            guard let existingObject = existing as? [String: Any] else {
                throw HooksInstaller.InstallError(message: "\"statusLine\" in \(settingsURL.path) is not a JSON object; fix or remove it and re-run.")
            }
            if Self.isOurs(existingObject, executablePath: executablePath) {
                let desired = ourStatusLineObject(preservingExtrasFrom: existingObject)
                if NSDictionary(dictionary: existingObject).isEqual(to: desired) {
                    report.action = "already installed"
                    report.passthroughCommand = StatusLineCommand.storedPassthroughCommand(settingsPath: passthroughSettingsPath)
                    return report
                }
                // Same feature, binary moved/renamed: update the command, keep the stored passthrough.
                settings["statusLine"] = desired
                report.action = "updated command path"
                report.passthroughCommand = StatusLineCommand.storedPassthroughCommand(settingsPath: passthroughSettingsPath)
            } else {
                let existingType = existingObject["type"] as? String ?? "command"
                guard existingType == "command" else {
                    throw HooksInstaller.InstallError(message: "statusLine in \(settingsURL.path) has type \"\(existingType)\", which claude-airou cannot pass through. Remove it (or switch it to a command) and re-run.")
                }
                if let existingCommand = existingObject["command"] as? String, StatusLineCommand.isSelfInvocation(existingCommand) {
                    throw HooksInstaller.InstallError(message: "statusLine already invokes claude-airou (\(existingCommand)); refusing to store it as its own passthrough.")
                }
                try storePassthrough(existingObject)
                settings["statusLine"] = ourStatusLineObject(preservingExtrasFrom: existingObject)
                report.action = "installed (original status line kept as passthrough)"
                report.passthroughCommand = existingObject["command"] as? String
            }
        } else {
            try storePassthrough(nil)
            settings["statusLine"] = ourStatusLineObject(preservingExtrasFrom: nil)
            report.action = "installed (there was no status line before)"
        }

        report.backupPath = try backup()
        try writeSettings(settings)
        return report
    }

    // MARK: - Uninstall

    func uninstall() throws -> Report {
        var settings = try loadSettings()
        var report = Report(settingsPath: settingsURL.path, action: "")
        guard let existing = settings["statusLine"] as? [String: Any], Self.isOurs(existing, executablePath: executablePath) else {
            report.action = "nothing to do (status line is not claude-airou's)"
            return report
        }
        if StatusLineCommand.passthroughFileIsCorrupt(settingsPath: passthroughSettingsPath) {
            throw HooksInstaller.InstallError(message: "\(StatusLineCommand.passthroughFile(forSettingsPath: passthroughSettingsPath).path) is unreadable; not touching settings.statusLine so your original status line is not lost. Fix or delete that file (your original statusLine is also in the settings.json.claude-airou-backup-* files) and re-run.")
        }
        let original = StatusLineCommand.storedPassthroughObject(settingsPath: passthroughSettingsPath) ?? [:]
        // Extra keys the user may have changed while ours was installed win over the stored copy.
        let extras = existing.filter { !Self.replacedKeys.contains($0.key) }
        if let originalCommand = original["command"] as? String, !originalCommand.isEmpty {
            var restored = original
            for (key, value) in extras where restored[key] == nil { restored[key] = value }
            settings["statusLine"] = restored
            report.action = "restored the original status line"
            report.passthroughCommand = originalCommand
        } else {
            settings.removeValue(forKey: "statusLine")
            report.action = "removed (there was no status line before)"
        }
        report.backupPath = try backup()
        try writeSettings(settings)
        try? FileManager.default.removeItem(at: StatusLineCommand.passthroughFile(forSettingsPath: passthroughSettingsPath))
        return report
    }

    // MARK: - Helpers

    private func storePassthrough(_ object: [String: Any]?) throws {
        try AppPaths.ensureDirectoryExists(AppPaths.rootDirectory)
        let data = try JSONSerialization.data(withJSONObject: object ?? [:], options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes])
        try data.write(to: StatusLineCommand.passthroughFile(forSettingsPath: passthroughSettingsPath), options: .atomic)
    }

    private func loadSettings() throws -> [String: Any] {
        guard FileManager.default.fileExists(atPath: settingsURL.path) else { return [:] }
        let data = try Data(contentsOf: settingsURL)
        if data.isEmpty { return [:] }
        guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw HooksInstaller.InstallError(message: "\(settingsURL.path) is not a JSON object; refusing to modify it.")
        }
        return object
    }

    private func backup() throws -> String? {
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
        let data = try JSONSerialization.data(withJSONObject: settings, options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes])
        try (data + Data("\n".utf8)).write(to: settingsURL, options: .atomic)
    }
}
