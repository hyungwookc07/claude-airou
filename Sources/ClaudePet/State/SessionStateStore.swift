import Foundation

/// File-based state exchange between the hook process (writer) and the overlay app (reader).
/// One JSON file per Claude Code session under `~/.claude-pet/state/`.
/// Files are the transport on purpose: the hook must never block or fail, and the overlay
/// may not even be running when a hook fires.
struct SessionStateStore {
    static let staleAfterSeconds: TimeInterval = 24 * 60 * 60

    let directory: URL

    init(directory: URL = AppPaths.stateDirectory) {
        self.directory = directory
    }

    private static let jsonEncoder: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        return encoder
    }()

    private static let jsonDecoder = JSONDecoder()

    // MARK: - Writing (hook side)

    func write(_ snapshot: SessionSnapshot) throws {
        try AppPaths.ensureDirectoryExists(directory)
        let data = try Self.jsonEncoder.encode(snapshot)
        // `.atomic` writes to a temporary file in the same directory and renames it into place.
        try data.write(to: fileURL(forSessionId: snapshot.sessionId), options: .atomic)
    }

    func remove(sessionId: String) {
        try? FileManager.default.removeItem(at: fileURL(forSessionId: sessionId))
    }

    /// The last snapshot written for a session (used by the hook to merge, not just overwrite).
    func read(sessionId: String) -> SessionSnapshot? {
        guard let data = try? Data(contentsOf: fileURL(forSessionId: sessionId)) else { return nil }
        return try? Self.jsonDecoder.decode(SessionSnapshot.self, from: data)
    }

    // MARK: - Reading (overlay side)

    /// Loads every readable snapshot. Files whose modification date is older than
    /// `staleAfterSeconds` are deleted (by mtime, so this works even if the file cannot be
    /// decoded); undecodable-but-fresh files are skipped, never deleted.
    func loadAll() -> [SessionSnapshot] {
        guard let entries = try? FileManager.default.contentsOfDirectory(
            at: directory,
            includingPropertiesForKeys: [.contentModificationDateKey],
            options: [.skipsHiddenFiles]
        ) else {
            return []
        }

        var snapshots: [SessionSnapshot] = []
        for fileURL in entries where fileURL.pathExtension == "json" {
            if let modified = try? fileURL.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate,
               Date().timeIntervalSince(modified) > Self.staleAfterSeconds {
                try? FileManager.default.removeItem(at: fileURL)
                continue
            }
            guard let data = try? Data(contentsOf: fileURL),
                  let snapshot = try? Self.jsonDecoder.decode(SessionSnapshot.self, from: data) else {
                continue
            }
            snapshots.append(snapshot)
        }
        return snapshots.sorted { $0.updatedAtEpochSeconds > $1.updatedAtEpochSeconds }
    }

    func removeAll() {
        for snapshot in loadAll() {
            remove(sessionId: snapshot.sessionId)
        }
    }

    // MARK: - Helpers

    func fileURL(forSessionId sessionId: String) -> URL {
        directory.appendingPathComponent(Self.sanitizeSessionId(sessionId) + ".json")
    }

    /// Session ids are UUIDs in practice, but never trust an id as a filename.
    static func sanitizeSessionId(_ sessionId: String) -> String {
        let allowed = Set("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_")
        let cleaned = String(sessionId.filter { allowed.contains($0) })
        let clipped = String(cleaned.prefix(80))
        return clipped.isEmpty ? "unknown-session" : clipped
    }
}
