import SwiftUI

/// The whole overlay content: a row of session cards (one pet, or one per session when fanned
/// out) with the speech bubble floating above the primary card.
struct PetView: View {
    @ObservedObject var model: PetViewModel

    var body: some View {
        let layout = model.layout
        let cardHeight = layout.cardHeight
        let cardsCenterY = RowLayout.speechBubbleReservedHeight + cardHeight / 2

        ZStack(alignment: .topLeading) {
            ForEach(layout.cards) { card in
                SessionCardView(model: model, card: card, cardHeight: cardHeight)
                    .frame(width: card.width, height: cardHeight)
                    .position(x: card.centerX, y: cardsCenterY)
            }

            speechBubbleArea
                .padding(.bottom, RowLayout.speechBubbleBottomInset)
                .frame(width: bubbleWidth(in: layout), height: RowLayout.speechBubbleReservedHeight, alignment: .bottom)
                .position(x: bubbleCenterX(in: layout), y: RowLayout.speechBubbleReservedHeight / 2)
        }
        .frame(width: layout.contentSize.width, height: layout.contentSize.height)
        // No implicit animation on layout changes: the panel is moved so the primary pet stays put on
        // screen, and animating card positions on top of that would make it visibly jump and slide.
    }

    private func bubbleWidth(in layout: RowLayout) -> CGFloat {
        min(max(layout.speechBubbleWidth, 40), layout.contentSize.width - 8)
    }

    /// Centre the bubble over the primary pet (RowLayout reserves the room), clamped as a safety net.
    private func bubbleCenterX(in layout: RowLayout) -> CGFloat {
        let half = bubbleWidth(in: layout) / 2
        return min(max(layout.primaryCenterX, half + 4), layout.contentSize.width - half - 4)
    }

    // MARK: - Speech bubble

    private var speechBubbleArea: some View {
        ZStack(alignment: .bottom) {
            if model.isSpeechBubbleVisible {
                SpeechBubble(text: model.speechBubbleText)
                    .transition(.opacity.combined(with: .scale(scale: 0.92, anchor: .bottom)))
            }
        }
        .animation(.easeOut(duration: 0.16), value: model.isSpeechBubbleVisible)
        .animation(.easeOut(duration: 0.16), value: model.speechBubbleText)
    }
}

// MARK: - One session card (sprite + status badge + label)

struct SessionCardView: View {
    @ObservedObject var model: PetViewModel
    let card: RowLayout.Card
    let cardHeight: CGFloat

    // Animated "from" state: cards start where they were on screen (or inside the primary pet)
    // and settle into their new slot. Screen-space, so the panel's own move never shows.
    @State private var animatedOffsetX: CGFloat = 0
    @State private var animatedScale: CGFloat = 1
    @State private var animatedOpacity: CGFloat = 1

    private static let settleAnimation: Animation = .spring(duration: 0.34, bounce: 0.18)
    private static let foldAnimation: Animation = .easeIn(duration: PetViewModel.collapseAnimationSeconds)

    private var spriteSize: CGSize {
        let grid = model.pet.gridSize
        return CGSize(width: CGFloat(grid.width) * card.pixelScale, height: CGFloat(grid.height) * card.pixelScale)
    }

    var body: some View {
        let state = model.state(for: card)
        VStack(spacing: 4) {
            Spacer(minLength: 0)
            sprite(state: state)
                .frame(width: spriteSize.width, height: spriteSize.height)
            if model.showsGauge {
                BatteryGauge(
                    remainingPercentage: model.gaugeValue(for: card),
                    label: model.gaugeMetric.shortLabel,
                    isCompact: !card.isPrimary
                )
                .frame(height: RowLayout.gaugeReservedHeight - 4)
            }
            if card.isPrimary {
                SessionBadge(
                    text: model.isExpanded ? card.label : model.collapsedLabel,
                    state: state,
                    isDimmed: model.focusedSession == nil,
                    isHighlighted: model.isExpanded,
                    hasAttentionDot: !model.isExpanded && model.hasHiddenAttention
                )
                .frame(height: RowLayout.sessionBadgeReservedHeight)
            } else {
                SessionBadge(text: card.label, state: state, isDimmed: false, isHighlighted: false, hasAttentionDot: false)
                    .frame(height: RowLayout.sessionBadgeReservedHeight)
            }
        }
        .frame(height: cardHeight, alignment: .bottom)
        .scaleEffect(animatedScale, anchor: .bottom)
        .offset(x: animatedOffsetX)
        .opacity(Double(animatedOpacity) * (card.isPrimary ? 1 : 0.92))
        .onAppear { animateFromPreviousLayout() }
        .onChange(of: model.layoutGeneration) { _, _ in animateFromPreviousLayout() }
        .onChange(of: model.isCollapsing) { _, isCollapsing in
            guard isCollapsing, !card.isPrimary else { return }
            // Fold back into the primary pet; the row collapses right after.
            withAnimation(Self.foldAnimation) {
                animatedOffsetX = model.layout.primaryCenterX - card.centerX
                animatedScale = 0.45
                animatedOpacity = 0
            }
        }
    }

    /// FLIP: place the card at its previous on-screen position (or inside the previous primary if
    /// it is new), then animate to its slot. All in screen space: `panelShiftX` is how far the
    /// panel itself just moved.
    private func animateFromPreviousLayout() {
        guard let previous = model.previousLayout else { return }
        let shift = model.panelShiftX
        let newScreenCenterX = card.centerX + shift
        let startOffsetX: CGFloat
        let startScale: CGFloat
        let startOpacity: CGFloat
        if let old = previous.cards.first(where: { $0.id == card.id }) {
            startOffsetX = old.centerX - newScreenCenterX
            startScale = old.pixelScale / card.pixelScale
            startOpacity = 1
        } else {
            startOffsetX = previous.primaryCenterX - newScreenCenterX
            startScale = 0.45
            startOpacity = 0
        }
        // Nothing to do when the card is exactly where it was (typically the primary).
        if abs(startOffsetX) < 0.5, abs(startScale - 1) < 0.01, startOpacity == 1 {
            animatedOffsetX = 0; animatedScale = 1; animatedOpacity = 1
            return
        }
        // Render the "from" state this frame, then animate to the slot on the next turn of the loop
        // (a synchronous withAnimation right after the assignment would be coalesced away).
        var transaction = Transaction(); transaction.disablesAnimations = true
        withTransaction(transaction) {
            animatedOffsetX = startOffsetX
            animatedScale = startScale
            animatedOpacity = startOpacity
        }
        DispatchQueue.main.async {
            withAnimation(Self.settleAnimation) {
                animatedOffsetX = 0
                animatedScale = 1
                animatedOpacity = 1
            }
        }
    }

    @ViewBuilder
    private func sprite(state: PetState) -> some View {
        let canvas = SpriteCanvas(frame: model.frame(for: state), palette: model.palette, pixelScale: card.pixelScale)
        ZStack(alignment: .topTrailing) {
            if card.isPrimary {
                canvas
                    .phaseAnimator([0.0, -14.0, 0.0, -7.0, 0.0], trigger: model.doneBounceTrigger) { content, offsetY in
                        content.offset(y: offsetY)
                    } animation: { _ in
                        .spring(duration: 0.16, bounce: 0.25)
                    }
                    .phaseAnimator([0.0, -5.0, 5.0, -4.0, 4.0, 0.0], trigger: model.errorShakeTrigger) { content, offsetX in
                        content.offset(x: offsetX)
                    } animation: { _ in
                        .linear(duration: 0.06)
                    }
            } else {
                canvas
            }

            if card.isPrimary {
                FloatingHeart(trigger: model.petReactionTrigger)
                    .frame(maxWidth: .infinity, alignment: .center)
                    .offset(y: -6)
            }
        }
    }
}

// MARK: - Sprite canvas

struct SpriteCanvas: View, Equatable {
    let frame: [String]
    let palette: ResolvedPalette
    let pixelScale: CGFloat

    nonisolated static func == (lhs: SpriteCanvas, rhs: SpriteCanvas) -> Bool {
        lhs.frame == rhs.frame && lhs.pixelScale == rhs.pixelScale && lhs.palette.colorsByCharacter == rhs.palette.colorsByCharacter
    }

    var body: some View {
        Canvas(opaque: false, rendersAsynchronously: false) { context, _ in
            for (rowIndex, row) in frame.enumerated() {
                for (columnIndex, character) in row.enumerated() {
                    guard let color = palette.color(for: character) else { continue }
                    let rect = CGRect(
                        x: CGFloat(columnIndex) * pixelScale,
                        y: CGFloat(rowIndex) * pixelScale,
                        width: pixelScale,
                        height: pixelScale
                    )
                    context.fill(Path(rect), with: .color(Color(color)))
                }
            }
        }
    }
}

extension Color {
    init(_ pixelColor: PixelColor) {
        self.init(.sRGB, red: pixelColor.red, green: pixelColor.green, blue: pixelColor.blue, opacity: pixelColor.alpha)
    }
}

// MARK: - Speech bubble

struct SpeechBubble: View {
    let text: String

    private var bubbleFill: Color { Color(nsColor: .windowBackgroundColor) }

    var body: some View {
        Text(text)
            .font(.system(size: 11.5, weight: .medium))
            .multilineTextAlignment(.leading) // one line: irrelevant; two lines: reads better ragged-right
            .lineLimit(2)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 9)
            .padding(.vertical, 6)
            .background(
                RoundedRectangle(cornerRadius: 9, style: .continuous)
                    .fill(bubbleFill)
                    .shadow(color: .black.opacity(0.18), radius: 3, y: 1)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 9, style: .continuous)
                    .strokeBorder(Color.primary.opacity(0.12))
            )
            .overlay(alignment: .bottom) {
                BubbleTail()
                    .fill(bubbleFill)
                    .frame(width: 12, height: 6)
                    .offset(y: 5.5)
            }
            .frame(maxWidth: RowLayout.speechBubbleMaxWidth)
    }
}

struct BubbleTail: Shape {
    func path(in rect: CGRect) -> Path {
        var path = Path()
        path.move(to: CGPoint(x: rect.minX, y: rect.minY))
        path.addLine(to: CGPoint(x: rect.maxX, y: rect.minY))
        path.addLine(to: CGPoint(x: rect.midX, y: rect.maxY))
        path.closeSubpath()
        return path
    }
}

// MARK: - Status badge (the Codex-style red clock / green check)

struct StatusBadge: View {
    let state: PetState
    var isCompact = false

    var body: some View {
        switch state {
        case .waitingApproval:
            BadgeIcon(systemName: "clock.fill", tint: .red, isPulsing: true, isSpinning: false, isCompact: isCompact)
        case .needsInput:
            BadgeIcon(systemName: "questionmark.circle.fill", tint: .orange, isPulsing: true, isSpinning: false, isCompact: isCompact)
        case .done:
            BadgeIcon(systemName: "checkmark.circle.fill", tint: .green, isPulsing: false, isSpinning: false, isCompact: isCompact)
        case .error:
            BadgeIcon(systemName: "exclamationmark.triangle.fill", tint: .red, isPulsing: false, isSpinning: false, isCompact: isCompact)
        case .working:
            BadgeIcon(systemName: "gearshape.fill", tint: .blue, isPulsing: false, isSpinning: true, isCompact: isCompact)
        case .thinking:
            // The sprite's own thinking frames (thought dots, eyes up) carry this state; a badge would be redundant.
            EmptyView()
        case .hello:
            BadgeIcon(systemName: "hand.wave.fill", tint: .yellow, isPulsing: false, isSpinning: false, isCompact: isCompact)
        case .idle:
            EmptyView()
        }
    }
}

struct BadgeIcon: View {
    let systemName: String
    let tint: Color
    let isPulsing: Bool
    let isSpinning: Bool
    var isCompact = false

    @State private var isRotating = false

    var body: some View {
        Image(systemName: systemName)
            .font(.system(size: isCompact ? 10 : 13, weight: .bold))
            .foregroundStyle(.white, tint)
            .symbolRenderingMode(.palette)
            .padding(isCompact ? 2 : 3)
            .background(Circle().fill(tint))
            .overlay(Circle().strokeBorder(.white.opacity(0.9), lineWidth: 1.5))
            .shadow(color: .black.opacity(0.25), radius: 2, y: 1)
            .symbolEffect(.pulse, options: .repeating, isActive: isPulsing)
            .rotationEffect(.degrees(isSpinning && isRotating ? 360 : 0))
            .animation(isSpinning ? .linear(duration: 2.4).repeatForever(autoreverses: false) : .default, value: isRotating)
            .onAppear { if isSpinning { isRotating = true } }
            .transition(.scale.combined(with: .opacity))
    }
}

/// A heart that floats up and fades when the user clicks the pet.
struct FloatingHeart: View {
    let trigger: Int
    @State private var progress: CGFloat = 1

    var body: some View {
        Image(systemName: "heart.fill")
            .font(.system(size: 14))
            .foregroundStyle(.pink)
            .offset(y: -28 * progress)
            .opacity(progress >= 1 ? 0 : 1 - Double(progress) * 0.9)
            .onChange(of: trigger) { _, _ in
                progress = 0
                DispatchQueue.main.async {
                    withAnimation(.easeOut(duration: 1.1)) { progress = 1 }
                }
            }
    }
}

// MARK: - Session badge (project name + status icon)

struct SessionBadge: View {
    let text: String
    var state: PetState = .idle
    let isDimmed: Bool
    var isHighlighted = false
    var hasAttentionDot = false

    var body: some View {
        HStack(spacing: 4) {
            if hasAttentionDot {
                Circle().fill(.red).frame(width: 6, height: 6)
            }
            Text(text)
                .font(.system(size: 9.5, weight: .semibold, design: .rounded))
                .foregroundStyle(isDimmed ? .tertiary : .secondary)
                .lineLimit(1)
            StatusBadge(state: state, isCompact: true)
                .animation(.spring(duration: 0.25), value: state)
        }
        .padding(.leading, 7)
        .padding(.trailing, state == .idle || state == .thinking ? 7 : 4)
        .padding(.vertical, 2.5)
        .background(Capsule().fill(Color(nsColor: .windowBackgroundColor).opacity(0.85)))
        .overlay(Capsule().strokeBorder(isHighlighted ? Color.accentColor.opacity(0.7) : Color.primary.opacity(0.1), lineWidth: isHighlighted ? 1.2 : 1))
        .frame(maxWidth: 200)
    }
}

// MARK: - Battery gauge (context / rate-limit remaining)

struct BatteryGauge: View {
    let remainingPercentage: Double?
    let label: String
    var isCompact = false

    private var fraction: CGFloat {
        CGFloat(max(0, min(100, remainingPercentage ?? 0)) / 100)
    }

    private var fillColor: Color {
        guard let remaining = remainingPercentage else { return .gray }
        if remaining <= 15 { return .red }
        if remaining <= 40 { return .yellow }
        return .green
    }

    private var bodyWidth: CGFloat { isCompact ? 18 : 24 }
    private var bodyHeight: CGFloat { isCompact ? 8 : 10 }

    var body: some View {
        HStack(spacing: 3) {
            HStack(spacing: 1) {
                ZStack(alignment: .leading) {
                    RoundedRectangle(cornerRadius: 2, style: .continuous)
                        .strokeBorder(Color.primary.opacity(0.45), lineWidth: 1)
                        .frame(width: bodyWidth, height: bodyHeight)
                    RoundedRectangle(cornerRadius: 1, style: .continuous)
                        .fill(fillColor)
                        .frame(width: max(0, (bodyWidth - 4) * fraction), height: bodyHeight - 4)
                        .padding(.leading, 2)
                        .animation(.easeOut(duration: 0.4), value: fraction)
                }
                RoundedRectangle(cornerRadius: 0.5)
                    .fill(Color.primary.opacity(0.45))
                    .frame(width: 1.5, height: bodyHeight * 0.45)
            }
            Text(remainingPercentage.map { "\(Int($0.rounded()))%" } ?? "–")
                .font(.system(size: isCompact ? 8 : 9, weight: .semibold, design: .rounded).monospacedDigit())
                .foregroundStyle(remainingPercentage == nil ? .tertiary : .secondary)
            if !isCompact, !label.isEmpty {
                Text(label)
                    .font(.system(size: 7.5, weight: .bold, design: .rounded))
                    .foregroundStyle(.tertiary)
            }
        }
        .padding(.horizontal, 5)
        .padding(.vertical, 1.5)
        // Same pill as the session badge so the gauge stays legible over any wallpaper.
        .background(Capsule().fill(Color(nsColor: .windowBackgroundColor).opacity(0.85)))
        .overlay(Capsule().strokeBorder(Color.primary.opacity(0.1)))
    }
}
