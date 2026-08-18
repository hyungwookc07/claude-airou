import Foundation

/// Usage figures for one session, fed by the Claude Code status line (`claude-airou statusline`)
/// or, when no status line runs (e.g. some desktop-app sessions), estimated from the transcript by
/// the hook. Stored next to the state file as `<session>.usage.json`.
struct SessionUsageSnapshot: Codable, Equatable {
    enum Source: String, Codable {
        case statusLine = "status_line"
        case transcript
    }

    var sessionId: String
    var source: Source
    var updatedAtEpochSeconds: Double

    /// Percent of the context window in use (0–100).
    var contextUsedPercentage: Double?
    var contextWindowSize: Int?
    var contextTokens: Int?
    var totalInputTokens: Int?
    var totalOutputTokens: Int?
    var modelDisplayName: String?

    /// Subscription rate limits (only the status line knows these).
    var fiveHourUsedPercentage: Double?
    var fiveHourResetsAtEpochSeconds: Double?
    var sevenDayUsedPercentage: Double?
    var sevenDayResetsAtEpochSeconds: Double?

    var totalCostUSD: Double?

    var contextRemainingPercentage: Double? {
        contextUsedPercentage.map { max(0, min(100, 100 - $0)) }
    }

    var fiveHourRemainingPercentage: Double? {
        fiveHourUsedPercentage.map { max(0, min(100, 100 - $0)) }
    }

    var sevenDayRemainingPercentage: Double? {
        sevenDayUsedPercentage.map { max(0, min(100, 100 - $0)) }
    }

    var updatedAt: Date { Date(timeIntervalSince1970: updatedAtEpochSeconds) }

    /// The status line is authoritative; a transcript estimate must not overwrite a recent status-line reading.
    static let statusLineAuthorityWindowSeconds: TimeInterval = 120
    /// Identical transcript estimates are re-written at most this often (keeps the file's mtime fresh
    /// for stale pruning without churning the overlay).
    static let transcriptRewriteIntervalSeconds: TimeInterval = 300

    /// Fields the transcript estimate cannot know; carried over from the previous snapshot.
    private mutating func inheritStatusLineOnlyFields(from previous: SessionUsageSnapshot) {
        if fiveHourUsedPercentage == nil { fiveHourUsedPercentage = previous.fiveHourUsedPercentage }
        if fiveHourResetsAtEpochSeconds == nil { fiveHourResetsAtEpochSeconds = previous.fiveHourResetsAtEpochSeconds }
        if sevenDayUsedPercentage == nil { sevenDayUsedPercentage = previous.sevenDayUsedPercentage }
        if sevenDayResetsAtEpochSeconds == nil { sevenDayResetsAtEpochSeconds = previous.sevenDayResetsAtEpochSeconds }
        if totalCostUSD == nil { totalCostUSD = previous.totalCostUSD }
        if totalInputTokens == nil { totalInputTokens = previous.totalInputTokens }
        if totalOutputTokens == nil { totalOutputTokens = previous.totalOutputTokens }
        if modelDisplayName == nil { modelDisplayName = previous.modelDisplayName }
    }

    /// True when the two snapshots carry the same figures (timestamps ignored).
    func hasSameFigures(as other: SessionUsageSnapshot) -> Bool {
        var a = self; var b = other
        a.updatedAtEpochSeconds = 0; b.updatedAtEpochSeconds = 0
        return a == b
    }

    /// What should be on disk after `candidate` arrives on top of `self`; nil = leave the file alone.
    func merged(with candidate: SessionUsageSnapshot, now: Date = Date()) -> SessionUsageSnapshot? {
        var result = candidate
        switch (source, candidate.source) {
        case (.statusLine, .transcript):
            if now.timeIntervalSince(updatedAt) < Self.statusLineAuthorityWindowSeconds { return nil }
            // Keep the status line's window (and rate limits, cost…): recompute the percentage against it.
            if let window = contextWindowSize, window > 0, let tokens = candidate.contextTokens {
                result.contextWindowSize = window
                result.contextUsedPercentage = min(100, Double(tokens) / Double(window) * 100)
            }
            result.inheritStatusLineOnlyFields(from: self)
        case (.transcript, .transcript):
            if hasSameFigures(as: candidate), now.timeIntervalSince(updatedAt) < Self.transcriptRewriteIntervalSeconds { return nil }
            result.inheritStatusLineOnlyFields(from: self)
        case (.transcript, .statusLine), (.statusLine, .statusLine):
            if result.contextWindowSize == nil { result.contextWindowSize = contextWindowSize }
            if result.modelDisplayName == nil { result.modelDisplayName = modelDisplayName }
        }
        return result
    }
}

/// Which figure the battery gauge shows.
enum GaugeMetric: String, Codable, CaseIterable {
    case contextRemaining = "context_remaining"
    case fiveHourRemaining = "five_hour_remaining"
    case sevenDayRemaining = "seven_day_remaining"
    case off

    var menuTitle: String {
        switch self {
        case .contextRemaining: return "Context window remaining"
        case .fiveHourRemaining: return "5-hour limit remaining"
        case .sevenDayRemaining: return "7-day limit remaining"
        case .off: return "Off"
        }
    }

    var shortLabel: String {
        switch self {
        case .contextRemaining: return "ctx"
        case .fiveHourRemaining: return "5h"
        case .sevenDayRemaining: return "7d"
        case .off: return ""
        }
    }

    func value(from usage: SessionUsageSnapshot?) -> Double? {
        guard let usage else { return nil }
        switch self {
        case .contextRemaining: return usage.contextRemainingPercentage
        case .fiveHourRemaining: return usage.fiveHourRemainingPercentage
        case .sevenDayRemaining: return usage.sevenDayRemainingPercentage
        case .off: return nil
        }
    }
}
