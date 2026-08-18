import Foundation

/// All on-disk locations used by claude-pet. Everything lives under `~/.claude-pet`
/// (override with `CLAUDE_PET_HOME`).
enum AppPaths {
    static var rootDirectory: URL {
        if let override = ProcessInfo.processInfo.environment["CLAUDE_PET_HOME"], !override.isEmpty {
            let expanded = (override as NSString).expandingTildeInPath
            return URL(fileURLWithPath: expanded, isDirectory: true).standardizedFileURL.absoluteURL
        }
        return FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".claude-pet", isDirectory: true)
    }

    static var stateDirectory: URL { rootDirectory.appendingPathComponent("state", isDirectory: true) }
    static var petsDirectory: URL { rootDirectory.appendingPathComponent("pets", isDirectory: true) }
    static var configFile: URL { rootDirectory.appendingPathComponent("config.json") }
    static var hookLogFile: URL { rootDirectory.appendingPathComponent("hook.log") }
    /// `claude-pet snapshot` drops this file; the running overlay answers by writing `snapshotImageFile`.
    static var snapshotRequestFile: URL { rootDirectory.appendingPathComponent("snapshot.request") }
    static var snapshotImageFile: URL { rootDirectory.appendingPathComponent("snapshot.png") }
    static var overlayLockFile: URL { rootDirectory.appendingPathComponent("overlay.lock") }

    static var claudeSettingsFile: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".claude", isDirectory: true)
            .appendingPathComponent("settings.json")
    }

    static func ensureDirectoryExists(_ url: URL) throws {
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    }
}
