import AppKit
import Combine
import Foundation
import SwiftUI

/// Drives the overlay: polls the session state directory, decides which session the pet
/// represents, decays transient states, and advances sprite frames.
@MainActor
final class PetViewModel: ObservableObject {
    // Layout constants (points)
    static let speechBubbleReservedHeight: CGFloat = 58
    static let sessionBadgeReservedHeight: CGFloat = 22
    static let horizontalPadding: CGFloat = 16
    static let minimumContentWidth: CGFloat = 220
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
    @Published var pixelScale: CGFloat
    @Published var isSpeechBubbleHidden: Bool
    @Published private(set) var doneBounceTrigger: Int = 0
    @Published private(set) var errorShakeTrigger: Int = 0
    @Published private(set) var petReactionTrigger: Int = 0

    /// Fires whenever the content size might have changed (pet or scale) so the panel can resize.
    let contentSizeDidChange = PassthroughSubject<CGSize, Never>()

    private let stateStore: SessionStateStore
    private var timer: Timer?
    private var tickCount = 0
    private var frameAccumulatorSeconds: TimeInterval = 0
    private var petReactionExpiresAt: Date?
    private var previousDisplayState: PetState = .idle
    private var previousFocusedSessionId: String?

    init(pet: PetDefinition, pixelScale: CGFloat, isSpeechBubbleHidden: Bool, stateStore: SessionStateStore = SessionStateStore()) {
        self.pet = pet
        self.palette = ResolvedPalette(definition: pet)
        self.pixelScale = pixelScale
        self.isSpeechBubbleHidden = isSpeechBubbleHidden
        self.stateStore = stateStore
    }

    // MARK: - Derived layout

    var spriteSize: CGSize {
        let grid = pet.gridSize
        return CGSize(width: CGFloat(grid.width) * pixelScale, height: CGFloat(grid.height) * pixelScale)
    }

    var contentSize: CGSize {
        let sprite = spriteSize
        let width = max(Self.minimumContentWidth, sprite.width + Self.horizontalPadding * 2)
        let height = Self.speechBubbleReservedHeight + sprite.height + Self.sessionBadgeReservedHeight + 12
        return CGSize(width: width, height: height)
    }

    var currentFrame: [String] {
        let frames = pet.frames(for: displayState)
        guard !frames.isEmpty else { return [] }
        return frames[frameIndex % frames.count]
    }

    var isSpeechBubbleVisible: Bool {
        if petReactionMessage != nil { return true }
        if isSpeechBubbleHidden { return false }
        return !displayMessage.isEmpty
    }

    var speechBubbleText: String {
        petReactionMessage ?? displayMessage
    }

    var activeSessionCount: Int { sessions.count }

    var sessionBadgeText: String {
        guard let focusedSession else { return "no session" }
        let extra = sessions.count - 1
        return extra > 0 ? "\(focusedSession.projectName) +\(extra)" : focusedSession.projectName
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

        // Attention-needing sessions win, then busy ones, then the most recently updated one.
        let focused = loaded.first(where: { $0.effectiveState.isAttentionNeeded })
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
    }

    private func advanceFrameIfDue() {
        frameAccumulatorSeconds += Self.tickIntervalSeconds
        let frameDuration = 1 / pet.framesPerSecond
        if frameAccumulatorSeconds >= frameDuration {
            frameAccumulatorSeconds -= frameDuration
            frameIndex &+= 1
        }
    }

    // MARK: - User interaction

    /// The user clicked the pet: react with a phrase (does not touch session state).
    func petWasClicked() {
        petReactionMessage = pet.petPhrases.randomElement()
        petReactionExpiresAt = Date().addingTimeInterval(Self.petReactionDurationSeconds)
        petReactionTrigger &+= 1
    }

    private func expirePetReactionIfDue() {
        guard let expiresAt = petReactionExpiresAt, Date() >= expiresAt else { return }
        petReactionExpiresAt = nil
        petReactionMessage = nil
    }

    // MARK: - Configuration changes

    func select(pet newPet: PetDefinition) {
        pet = newPet
        palette = ResolvedPalette(definition: newPet)
        frameIndex = 0
        contentSizeDidChange.send(contentSize)
    }

    func setPixelScale(_ newScale: CGFloat) {
        pixelScale = newScale
        contentSizeDidChange.send(contentSize)
    }
}
