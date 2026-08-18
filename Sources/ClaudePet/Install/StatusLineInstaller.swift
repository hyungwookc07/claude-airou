import Foundation

/// Swaps `settings.statusLine` for `claude-pet statusline`, keeping the user's original status line
/// object in `~/.claude-pet/statusline-passthrough.json` so it keeps running (and can be restored).
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

    let settingsURL: URL
    let executablePath: String

    init(settingsURL: URL = AppPaths.claudeSettingsFile, executablePath: String? = nil) {
        self.settingsURL = settingsURL
        self.executablePath = executablePath ?? HooksInstaller.currentExecutablePath()
    }

    var ourCommand: String {
        "\(HooksInstaller.shellSingleQuoted(executablePath)) \(Self.subcommand)"
    }

    var ourStatusLineObject: [String: Any] {
        ["type": "command", "command": ourCommand]
    }

    static func isOurs(_ statusLine: [String: Any]) -> Bool {
        guard let command = statusLine["command"] as? String else { return false }
        return command.contains(HooksInstaller.commandMarker) && command.trimmingCharacters(in: .whitespaces).hasSuffix(" " + subcommand)
    }

    // MARK: - Install

    func install() throws -> Report {
        var settings = try loadSettings()
        var report = Report(settingsPath: settingsURL.path, action: "")

        if let existing = settings["statusLine"] {
            guard let existingObject = existing as? [String: Any] else {
                throw HooksInstaller.InstallError(message: "\"statusLine\" in \(settingsURL.path) is not a JSON object; fix or remove it and re-run.")
            }
            if Self.isOurs(existingObject) {
                let current = existingObject["command"] as? String
                if current == ourCommand {
                    report.action = "already installed"
                    report.passthroughCommand = StatusLineCommand.storedPassthroughCommand()
                    return report
                }
                // Same feature, binary moved: update the path, keep the stored passthrough.
                settings["statusLine"] = ourStatusLineObject
                report.action = "updated command path"
                report.passthroughCommand = StatusLineCommand.storedPassthroughCommand()
            } else {
                try storePassthrough(existingObject)
                settings["statusLine"] = ourStatusLineObject
                report.action = "installed (original status line kept as passthrough)"
                report.passthroughCommand = existingObject["command"] as? String
            }
        } else {
            try storePassthrough(nil)
            settings["statusLine"] = ourStatusLineObject
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
        guard let existing = settings["statusLine"] as? [String: Any], Self.isOurs(existing) else {
            report.action = "nothing to do (status line is not claude-pet's)"
            return report
        }
        if let original = StatusLineCommand.storedPassthroughObject(), !original.isEmpty {
            settings["statusLine"] = original
            report.action = "restored the original status line"
            report.passthroughCommand = original["command"] as? String
        } else {
            settings.removeValue(forKey: "statusLine")
            report.action = "removed (there was no status line before)"
        }
        report.backupPath = try backup()
        try writeSettings(settings)
        try? FileManager.default.removeItem(at: StatusLineCommand.passthroughFile)
        return report
    }

    // MARK: - Helpers

    private func storePassthrough(_ object: [String: Any]?) throws {
        try AppPaths.ensureDirectoryExists(AppPaths.rootDirectory)
        let data = try JSONSerialization.data(withJSONObject: object ?? [:], options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes])
        try data.write(to: StatusLineCommand.passthroughFile, options: .atomic)
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
        let baseName = "\(settingsURL.lastPathComponent).claude-pet-backup-\(formatter.string(from: Date()))"
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
