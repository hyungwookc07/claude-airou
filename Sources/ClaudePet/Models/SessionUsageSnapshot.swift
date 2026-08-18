import Foundation

/// Usage figures for one session, fed by the Claude Code status line (`claude-pet statusline`)
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

    func shouldBeReplaced(by candidate: SessionUsageSnapshot, now: Date = Date()) -> Bool {
        if candidate.source == .statusLine { return true }
        if source == .statusLine, now.timeIntervalSince(updatedAt) < Self.statusLineAuthorityWindowSeconds {
            return false
        }
        return true
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
