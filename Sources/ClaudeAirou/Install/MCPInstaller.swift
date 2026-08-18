import Foundation

/// Adds/removes the `claude-airou` MCP server entry in the Claude desktop app's
/// `claude_desktop_config.json`, so Claude chat can drive the pet.
///
/// Same manners as `HooksInstaller`: only our own `mcpServers["claude-airou"]` key is
/// touched, a timestamped backup is written before any change, and the operation is
/// idempotent (re-running updates the command path instead of duplicating anything).
struct MCPInstaller {
    static let serverKey = "claude-airou"
    static let serverSubcommand = "mcp"

    struct Report {
        enum Outcome {
            case added, updated, unchanged, removed, absent
        }

        var configPath: String
        var backupPath: String?
        var outcome: Outcome = .unchanged
        var serverCommand: String

        var summaryText: String {
            var lines: [String] = []
            lines.append("Config:  \(configPath)")
            if let backupPath { lines.append("Backup:  \(backupPath)") }
            lines.append("Command: \(serverCommand)")
            switch outcome {
            case .added: lines.append("Added:   mcpServers.\(MCPInstaller.serverKey)")
            case .updated: lines.append("Updated: mcpServers.\(MCPInstaller.serverKey)")
            case .unchanged: lines.append("Already installed; nothing changed.")
            case .removed: lines.append("Removed: mcpServers.\(MCPInstaller.serverKey)")
            case .absent: lines.append("Not installed; nothing changed.")
            }
            return lines.joined(separator: "\n")
        }
    }

    let configURL: URL
    let executablePath: String

    init(configURL: URL = AppPaths.claudeDesktopConfigFile, executablePath: String? = nil) {
        self.configURL = configURL
        self.executablePath = executablePath ?? HooksInstaller.currentExecutablePath()
    }

    /// The entry the desktop app spawns: `<binary> mcp` (exec form, no shell involved).
    static func serverEntry(executablePath: String) -> [String: Any] {
        ["command": executablePath, "args": [serverSubcommand]]
    }

    // MARK: - Install

    func install() throws -> Report {
        var config = try loadConfig()
        var servers = try serversObject(from: config)
        var report = Report(configPath: configURL.path, serverCommand: "\(executablePath) \(Self.serverSubcommand)")
        let desired = Self.serverEntry(executablePath: executablePath)

        if let existing = servers[Self.serverKey] as? [String: Any] {
            if NSDictionary(dictionary: existing).isEqual(to: desired) {
                report.outcome = .unchanged
                return report
            }
            report.outcome = .updated
        } else {
            report.outcome = .added
        }

        servers[Self.serverKey] = desired
        config["mcpServers"] = servers
        report.backupPath = try backupIfNeeded()
        try writeConfig(config)
        return report
    }

    // MARK: - Uninstall

    func uninstall() throws -> Report {
        var config = try loadConfig()
        var report = Report(configPath: configURL.path, serverCommand: "\(executablePath) \(Self.serverSubcommand)")
        guard config["mcpServers"] != nil else {
            report.outcome = .absent
            return report
        }
        var servers = try serversObject(from: config)
        guard servers.removeValue(forKey: Self.serverKey) != nil else {
            report.outcome = .absent
            return report
        }
        report.outcome = .removed

        if servers.isEmpty {
            config.removeValue(forKey: "mcpServers")
        } else {
            config["mcpServers"] = servers
        }
        report.backupPath = try backupIfNeeded()
        try writeConfig(config)
        return report
    }

    // MARK: - Helpers

    /// `mcpServers` must be an object if present. Anything else is refused rather than clobbered.
    private func serversObject(from config: [String: Any]) throws -> [String: Any] {
        guard let raw = config["mcpServers"] else { return [:] }
        guard let object = raw as? [String: Any] else {
            throw HooksInstaller.InstallError(message: "\"mcpServers\" in \(configURL.path) is not a JSON object; fix or remove it and re-run.")
        }
        return object
    }

    private func loadConfig() throws -> [String: Any] {
        guard FileManager.default.fileExists(atPath: configURL.path) else { return [:] }
        let data = try Data(contentsOf: configURL)
        if data.isEmpty { return [:] }
        guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw HooksInstaller.InstallError(message: "\(configURL.path) is not a JSON object; refusing to modify it.")
        }
        return object
    }

    private func backupIfNeeded() throws -> String? {
        guard FileManager.default.fileExists(atPath: configURL.path) else { return nil }
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyyMMdd-HHmmss"
        let baseName = "\(configURL.lastPathComponent).claude-airou-backup-\(formatter.string(from: Date()))"
        let directory = configURL.deletingLastPathComponent()
        var backupURL = directory.appendingPathComponent(baseName)
        var suffix = 1
        while FileManager.default.fileExists(atPath: backupURL.path) {
            backupURL = directory.appendingPathComponent("\(baseName)-\(suffix)")
            suffix += 1
        }
        try FileManager.default.copyItem(at: configURL, to: backupURL)
        return backupURL.path
    }

    private func writeConfig(_ config: [String: Any]) throws {
        try AppPaths.ensureDirectoryExists(configURL.deletingLastPathComponent())
        let data = try JSONSerialization.data(
            withJSONObject: config,
            options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
        )
        try (data + Data("\n".utf8)).write(to: configURL, options: .atomic)
    }

    /// The JSON a user would paste by hand (printed by `claude-airou install-mcp --print`).
    func snippetJSON() -> String {
        let object: [String: Any] = ["mcpServers": [Self.serverKey: Self.serverEntry(executablePath: executablePath)]]
        let data = (try? JSONSerialization.data(
            withJSONObject: object,
            options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
        )) ?? Data()
        return String(decoding: data, as: UTF8.self)
    }
}
