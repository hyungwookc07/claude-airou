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
    /// `claude-pet click X` drops this file (content: x in points) so the overlay behaves as if clicked — for scripted testing.
    static var clickRequestFile: URL { rootDirectory.appendingPathComponent("click.request") }

    static var claudeSettingsFile: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".claude", isDirectory: true)
            .appendingPathComponent("settings.json")
    }

    static func ensureDirectoryExists(_ url: URL) throws {
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    }
}

/// Tiny append-only log for the overlay (clicks, layout) at `~/.claude-pet/overlay.log`; truncated when large.
enum OverlayLog {
    static let maxBytes: UInt64 = 256 * 1024

    static func append(_ line: String) {
        let url = AppPaths.rootDirectory.appendingPathComponent("overlay.log")
        do {
            try AppPaths.ensureDirectoryExists(url.deletingLastPathComponent())
            let fileManager = FileManager.default
            if let size = (try? fileManager.attributesOfItem(atPath: url.path))?[.size] as? UInt64, size > maxBytes {
                try? fileManager.removeItem(at: url)
            }
            if !fileManager.fileExists(atPath: url.path) {
                fileManager.createFile(atPath: url.path, contents: nil)
            }
            let handle = try FileHandle(forWritingTo: url)
            defer { try? handle.close() }
            try handle.seekToEnd()
            let formatter = DateFormatter()
            formatter.dateFormat = "HH:mm:ss.SSS"
            try handle.write(contentsOf: Data("\(formatter.string(from: Date())) \(line)\n".utf8))
        } catch {
            // best effort
        }
    }
}
