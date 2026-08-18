import AppKit
import Combine
import Foundation
import SwiftUI

/// Drives the overlay: polls the session state directory, decides which session the pet
/// represents, decays transient states, advances sprite frames, and lays out the row of
/// session cards (one pet when collapsed, one pet per session when fanned out).
@MainActor
final class PetViewModel: ObservableObject {
    static let tickIntervalSeconds: TimeInterval = 0.1
    static let stateReloadEveryTicks = 3
    static let petReactionDurationSeconds: TimeInterval = 2.5

    // Published for the SwiftUI view
    @Published private(set) var pet: PetDefinition
    @Published private(set) var palette: ResolvedPalette
    @Published private(set) var focusedSession: SessionSnapshot?
    @Published private(set) var sessions: [SessionSnapshot] = []
    @Published private(set) var displayState: PetState = .idle
    @Published private(set) var displayMessage: String = ""
    @Published private(set) var frameIndex: Int = 0
    @Published private(set) var petReactionMessage: String?
    @Published private(set) var pixelScale: CGFloat
    @Published var isSpeechBubbleHidden: Bool {
        didSet { relayoutIfNeeded() }
    }
    @Published private(set) var doneBounceTrigger: Int = 0
    @Published private(set) var errorShakeTrigger: Int = 0
    @Published private(set) var petReactionTrigger: Int = 0

    /// Fan-out: show every session side by side. Toggled by clicking the pet; forced by the menu.
    @Published private(set) var isFannedOut = false
    @Published var isAlwaysFannedOut: Bool {
        didSet { relayoutIfNeeded() }
    }
    /// User-chosen focus (click a side pet / pick from the menu). Overrides the automatic rule.
    @Published private(set) var pinnedSessionId: String?

    /// Usage per session (status line or transcript estimate) for the battery gauge.
    @Published private(set) var usageBySessionId: [String: SessionUsageSnapshot] = [:]
    @Published var gaugeMetric: GaugeMetric {
        didSet { relayoutIfNeeded() }
    }

    /// Current geometry; the panel resizes from this.
    @Published private(set) var layout: RowLayout
    /// Bumped on every layout change so cards can run their entrance / move animation.
    @Published private(set) var layoutGeneration = 0
    /// The layout before the last change (cards animate from their old screen position).
    private(set) var previousLayout: RowLayout?
    /// How far the panel moved (screen x, points) to keep the primary pet still during the last
    /// layout change; set by the app delegate right after resizing, read by the view.
    var panelShiftX: CGFloat = 0
    /// True while side cards fold back into the primary before the row actually collapses.
    @Published private(set) var isCollapsing = false
    static let collapseAnimationSeconds: TimeInterval = 0.22

    /// Fires whenever `layout` changed in a way that needs the panel resized (size or primary position).
    let layoutDidChange = PassthroughSubject<RowLayout, Never>()

    private let stateStore: SessionStateStore
    private var timer: Timer?
    private var tickCount = 0
    private var frameAccumulatorSeconds: TimeInterval = 0
    private var petReactionExpiresAt: Date?
    private var previousDisplayState: PetState = .idle
    private var previousFocusedSessionId: String?

    init(
        pet: PetDefinition,
        pixelScale: CGFloat,
        isSpeechBubbleHidden: Bool,
        isAlwaysFannedOut: Bool,
        gaugeMetric: GaugeMetric,
        stateStore: SessionStateStore = SessionStateStore()
    ) {
        self.pet = pet
        self.palette = ResolvedPalette(definition: pet)
        self.pixelScale = pixelScale
        self.isSpeechBubbleHidden = isSpeechBubbleHidden
        self.isAlwaysFannedOut = isAlwaysFannedOut
        self.gaugeMetric = gaugeMetric
        self.stateStore = stateStore
        self.layout = RowLayout.make(gridSize: pet.gridSize, pixelScale: pixelScale, labels: ["no session"], sessionIds: [nil], primaryIndex: 0, showsGauge: gaugeMetric != .off)
    }

    // MARK: - Gauge

    var showsGauge: Bool { gaugeMetric != .off }

    func usage(for card: RowLayout.Card) -> SessionUsageSnapshot? {
        guard let id = card.sessionId else { return nil }
        return usageBySessionId[id]
    }

    /// Remaining percentage for the gauge metric, or nil when unknown.
    func gaugeValue(for card: RowLayout.Card) -> Double? {
        gaugeMetric.value(from: usage(for: card))
    }

    /// One-line usage summary for menus: "ctx 62% left · 5h 71% left · $0.42".
    func usageSummary(for sessionId: String?) -> String? {
        guard let sessionId, let usage = usageBySessionId[sessionId] else { return nil }
        var parts: [String] = []
        if let ctx = usage.contextRemainingPercentage { parts.append("ctx \(Int(ctx.rounded()))% left") }
        if let five = usage.fiveHourRemainingPercentage { parts.append("5h \(Int(five.rounded()))% left") }
        if let seven = usage.sevenDayRemainingPercentage { parts.append("7d \(Int(seven.rounded()))% left") }
        if let cost = usage.totalCostUSD { parts.append(String(format: "$%.2f", cost)) }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    // MARK: - Speech bubble measurement

    static let speechBubbleFont = NSFont.systemFont(ofSize: 11.5, weight: .medium)
    static let speechBubbleHorizontalPadding: CGFloat = 9

    /// Width the bubble will take for `text` (single line up to the max, then it wraps to two lines).
    static func measuredSpeechBubbleWidth(for text: String) -> CGFloat {
        guard !text.isEmpty else { return 0 }
        let textWidth = (text as NSString).size(withAttributes: [.font: speechBubbleFont]).width
        return min(RowLayout.speechBubbleMaxWidth, (textWidth + speechBubbleHorizontalPadding * 2 + 2).rounded(.up))
    }

    var currentSpeechBubbleWidth: CGFloat {
        isSpeechBubbleVisible ? Self.measuredSpeechBubbleWidth(for: speechBubbleText) : 0
    }

    // MARK: - Derived

    var isExpanded: Bool { (isFannedOut || isAlwaysFannedOut) && sessions.count >= 2 }

    var contentSize: CGSize { layout.contentSize }

    /// Geometry of the collapsed (single-card) row for the current pet and scale — used to
    /// store a stable window position regardless of the current fan-out state.
    var collapsedLayout: RowLayout {
        RowLayout.make(
            gridSize: pet.gridSize,
            pixelScale: pixelScale,
            labels: [collapsedLabel],
            sessionIds: [focusedSession?.sessionId],
            primaryIndex: 0,
            speechBubbleWidth: currentSpeechBubbleWidth,
            showsGauge: showsGauge
        )
    }

    func frame(for state: PetState) -> [String] {
        let frames = pet.frames(for: state)
        guard !frames.isEmpty else { return [] }
        return frames[frameIndex % frames.count]
    }

    var currentFrame: [String] { frame(for: displayState) }

    var isSpeechBubbleVisible: Bool {
        if petReactionMessage != nil { return true }
        if isSpeechBubbleHidden { return false }
        return !displayMessage.isEmpty
    }

    var speechBubbleText: String { petReactionMessage ?? displayMessage }

    var activeSessionCount: Int { sessions.count }

    /// Label for the collapsed badge: "project" or "project +2".
    var collapsedLabel: String {
        guard let focusedSession else { return "no session" }
        let extra = sessions.count - 1
        return extra > 0 ? "\(focusedSession.projectName) +\(extra)" : focusedSession.projectName
    }

    /// True when a session other than the focused one is waiting on the user (shown as a red dot on the badge).
    var hasHiddenAttention: Bool {
        sessions.contains { $0.sessionId != focusedSession?.sessionId && $0.effectiveState.isAttentionNeeded }
    }

    func session(withId id: String?) -> SessionSnapshot? {
        guard let id else { return nil }
        return sessions.first { $0.sessionId == id }
    }

    /// The state to draw for a given card (transient decay applied).
    func state(for card: RowLayout.Card) -> PetState {
        if card.isPrimary { return displayState }
        return session(withId: card.sessionId)?.effectiveState ?? .idle
    }

    // MARK: - Lifecycle

    func start() {
        guard timer == nil else { return }
        reloadSessions()
        let timer = Timer(timeInterval: Self.tickIntervalSeconds, repeats: true) { [weak self] _ in
            Task { @MainActor [weak self] in self?.tick() }
        }
        RunLoop.main.add(timer, forMode: .common)
        self.timer = timer
    }

    func stop() {
        timer?.invalidate()
        timer = nil
    }

    private func tick() {
        tickCount &+= 1
        if tickCount % Self.stateReloadEveryTicks == 0 {
            reloadSessions()
        }
        advanceFrameIfDue()
        expirePetReactionIfDue()
    }

    // MARK: - Session selection

    func reloadSessions() {
        let loaded = stateStore.loadAll()
        // Only publish when something actually changed; this runs several times a second.
        if loaded != sessions { sessions = loaded }
        let loadedUsage = stateStore.loadAllUsage()
        if loadedUsage != usageBySessionId { usageBySessionId = loadedUsage }

        if let pinnedSessionId, !loaded.contains(where: { $0.sessionId == pinnedSessionId }) {
            self.pinnedSessionId = nil // the pinned session ended
        }
        if loaded.count < 2, isFannedOut {
            isFannedOut = false // nothing left to fan out
            isCollapsing = false
        }

        // Pinned session wins, then attention-needing, then busy, then the most recently updated.
        let focused = loaded.first(where: { $0.sessionId == pinnedSessionId })
            ?? loaded.first(where: { $0.effectiveState.isAttentionNeeded })
            ?? loaded.first(where: { $0.effectiveState.isBusy })
            ?? loaded.first
        if focused != focusedSession { focusedSession = focused }

        let newState = focused?.effectiveState ?? .idle
        let newMessage: String
        if let focused, newState != .idle {
            newMessage = focused.message
        } else {
            newMessage = ""
        }

        let focusChanged = focused?.sessionId != previousFocusedSessionId
        let stateChanged = newState != previousDisplayState
        if stateChanged || focusChanged {
            frameIndex = 0
            frameAccumulatorSeconds = 0
            if newState == .done || newState == .hello { doneBounceTrigger &+= 1 }
            if newState == .error { errorShakeTrigger &+= 1 }
        }
        previousDisplayState = newState
        previousFocusedSessionId = focused?.sessionId
        if newState != displayState { displayState = newState }
        if newMessage != displayMessage { displayMessage = newMessage }

        relayoutIfNeeded()
    }

    private func advanceFrameIfDue() {
        frameAccumulatorSeconds += Self.tickIntervalSeconds
        let frameDuration = 1 / pet.framesPerSecond
        if frameAccumulatorSeconds >= frameDuration {
            frameAccumulatorSeconds -= frameDuration
            frameIndex &+= 1
        }
    }

    // MARK: - Layout

    /// Row order when expanded: primary in the middle, the others alternating right/left by recency.
    private func expandedRow() -> [SessionSnapshot] {
        guard let focusedSession else { return [] }
        let others = sessions.filter { $0.sessionId != focusedSession.sessionId }
        var left: [SessionSnapshot] = []
        var right: [SessionSnapshot] = []
        for (index, session) in others.enumerated() {
            if index % 2 == 0 { right.append(session) } else { left.append(session) }
        }
        return left.reversed() + [focusedSession] + right
    }

    private func relayoutIfNeeded() {
        let newLayout: RowLayout
        if isExpanded {
            let row = expandedRow()
            let primaryIndex = row.firstIndex { $0.sessionId == focusedSession?.sessionId } ?? 0
            newLayout = RowLayout.make(
                gridSize: pet.gridSize,
                pixelScale: pixelScale,
                labels: row.map(\.projectName),
                sessionIds: row.map { Optional($0.sessionId) },
                primaryIndex: primaryIndex,
                speechBubbleWidth: currentSpeechBubbleWidth,
                showsGauge: showsGauge
            )
        } else {
            newLayout = collapsedLayout
        }
        guard newLayout != layout else { return }
        let needsPanelUpdate = newLayout.contentSize != layout.contentSize || newLayout.primaryCenterX != layout.primaryCenterX
        previousLayout = layout
        panelShiftX = 0
        layout = newLayout
        if needsPanelUpdate {
            layoutDidChange.send(newLayout) // synchronous: the delegate resizes the panel and sets panelShiftX
        }
        layoutGeneration &+= 1
    }

    // MARK: - User interaction

    /// A click at `contentX` (points from the panel's left edge).
    func handleClick(atContentX contentX: CGFloat) {
        let cardsDescription = layout.cards.map { "\($0.label)[\(Int($0.x))-\(Int($0.x + $0.width))]\($0.isPrimary ? "*" : "")" }.joined(separator: " ")
        func log(_ action: String) {
            OverlayLog.append("click x=\(Int(contentX)) width=\(Int(layout.contentSize.width)) sessions=\(sessions.count) expanded=\(isExpanded) cards: \(cardsDescription) -> \(action)")
        }
        guard sessions.count >= 2 else {
            log("pet")
            petWasClicked()
            return
        }
        if isCollapsing {
            log("ignored (collapsing)")
            return
        }
        if !isExpanded {
            log("expand")
            isFannedOut = true
            relayoutIfNeeded()
            return
        }
        guard let card = layout.card(atContentX: contentX) else {
            log("gap → collapse")
            collapse()
            return
        }
        if card.isPrimary {
            if isAlwaysFannedOut {
                log("primary → pet (always expanded)")
                petWasClicked() // can't collapse; treat as petting
            } else {
                log("primary → collapse")
                collapse()
            }
        } else if let sessionId = card.sessionId {
            log("pin \(card.label)")
            pin(sessionId: sessionId)
        }
    }

    /// Folds the side cards back into the primary, then collapses the row.
    func collapse() {
        guard isFannedOut, !isCollapsing else { return }
        isCollapsing = true
        DispatchQueue.main.asyncAfter(deadline: .now() + Self.collapseAnimationSeconds) { [weak self] in
            guard let self else { return }
            self.isCollapsing = false
            self.isFannedOut = false
            self.relayoutIfNeeded()
        }
    }

    /// Make a session the primary one, overriding the automatic focus rule. `nil` restores automatic.
    func pin(sessionId: String?) {
        pinnedSessionId = sessionId
        reloadSessions()
    }

    /// The user clicked the pet itself: react with a phrase (does not touch session state).
    func petWasClicked() {
        petReactionMessage = pet.petPhrases.randomElement()
        petReactionExpiresAt = Date().addingTimeInterval(Self.petReactionDurationSeconds)
        petReactionTrigger &+= 1
        relayoutIfNeeded()
    }

    private func expirePetReactionIfDue() {
        guard let expiresAt = petReactionExpiresAt, Date() >= expiresAt else { return }
        petReactionExpiresAt = nil
        petReactionMessage = nil
        relayoutIfNeeded()
    }

    // MARK: - Configuration changes

    func select(pet newPet: PetDefinition) {
        pet = newPet
        palette = ResolvedPalette(definition: newPet)
        frameIndex = 0
        relayoutIfNeeded()
    }

    func setPixelScale(_ newScale: CGFloat) {
        pixelScale = newScale
        relayoutIfNeeded()
    }
}
