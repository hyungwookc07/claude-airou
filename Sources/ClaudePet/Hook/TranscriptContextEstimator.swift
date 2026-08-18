import Foundation

/// Fallback usage source for sessions that never run a status line (e.g. some desktop-app
/// sessions): the last assistant message in the transcript carries `usage`, and
/// input + cache_creation + cache_read tokens is what Claude Code itself shows as context usage.
enum TranscriptContextEstimator {
    static let tailBytesToRead = 96 * 1024
    static let defaultContextWindowSize = 200_000
    static let largeContextWindowSize = 1_000_000

    /// Hook events after which the transcript may have a fresh assistant message.
    static let refreshingEventNames: Set<String> = ["UserPromptSubmit", "PostToolUse", "PostToolBatch", "Stop", "SessionStart", "PostCompact"]

    static func estimate(transcriptPath: String, sessionId: String, now: Date = Date()) -> SessionUsageSnapshot? {
        guard let handle = FileHandle(forReadingAtPath: transcriptPath) else { return nil }
        defer { try? handle.close() }
        guard let fileSize = try? handle.seekToEnd() else { return nil }
        let start = fileSize > UInt64(tailBytesToRead) ? fileSize - UInt64(tailBytesToRead) : 0
        guard (try? handle.seek(toOffset: start)) != nil, let data = try? handle.readToEnd(), !data.isEmpty else { return nil }

        // Walk lines from the end; the first (i.e. latest) assistant entry with usage wins.
        let lines = data.split(separator: UInt8(ascii: "\n"), omittingEmptySubsequences: true)
        for line in lines.reversed() {
            guard let object = try? JSONSerialization.jsonObject(with: Data(line)) as? [String: Any],
                  object["type"] as? String == "assistant",
                  let message = object["message"] as? [String: Any],
                  let usage = message["usage"] as? [String: Any] else { continue }
            func count(_ key: String) -> Int {
                if let int = usage[key] as? Int { return int }
                if let double = usage[key] as? Double { return Int(double) }
                return 0
            }
            let contextTokens = count("input_tokens") + count("cache_creation_input_tokens") + count("cache_read_input_tokens")
            guard contextTokens > 0 else { continue }
            let model = (message["model"] as? String) ?? ""
            let windowSize = contextWindowSize(forModel: model, observedContextTokens: contextTokens)
            var snapshot = SessionUsageSnapshot(sessionId: sessionId, source: .transcript, updatedAtEpochSeconds: now.timeIntervalSince1970)
            snapshot.contextTokens = contextTokens
            snapshot.contextWindowSize = windowSize
            snapshot.contextUsedPercentage = min(100, Double(contextTokens) / Double(windowSize) * 100)
            snapshot.modelDisplayName = model.isEmpty ? nil : model
            return snapshot
        }
        return nil
    }

    /// We cannot know every model's window; the transcript never says. Use the 1M tier when the
    /// model id says so or when the observed context already exceeds the 200k tier — the estimate
    /// is only for a gauge, so "somewhere in the bigger window" beats a pinned 100%.
    static func contextWindowSize(forModel model: String, observedContextTokens: Int = 0) -> Int {
        let lowered = model.lowercased()
        if lowered.contains("[1m]") || lowered.hasSuffix("-1m") || lowered.contains("1m-context") {
            return largeContextWindowSize
        }
        if observedContextTokens > defaultContextWindowSize {
            return largeContextWindowSize
        }
        return defaultContextWindowSize
    }
}
