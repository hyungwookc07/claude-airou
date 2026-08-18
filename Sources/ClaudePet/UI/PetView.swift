import SwiftUI

/// The whole overlay content: a row of session cards (one pet, or one per session when fanned
/// out) with the speech bubble floating above the primary card.
struct PetView: View {
    @ObservedObject var model: PetViewModel

    var body: some View {
        let layout = model.layout
        let cardHeight = layout.primarySpriteSize.height + RowLayout.sessionBadgeReservedHeight + 4
        let cardsCenterY = RowLayout.speechBubbleReservedHeight + cardHeight / 2

        ZStack(alignment: .topLeading) {
            ForEach(layout.cards) { card in
                SessionCardView(model: model, card: card, cardHeight: cardHeight)
                    .frame(width: card.width, height: cardHeight)
                    .position(x: card.centerX, y: cardsCenterY)
            }

            speechBubbleArea
                .frame(width: bubbleWidth(in: layout), height: RowLayout.speechBubbleReservedHeight, alignment: .bottom)
                .position(x: bubbleCenterX(in: layout), y: RowLayout.speechBubbleReservedHeight / 2)
        }
        .frame(width: layout.contentSize.width, height: layout.contentSize.height)
        // No implicit animation on layout changes: the panel is moved so the primary pet stays put on
        // screen, and animating card positions on top of that would make it visibly jump and slide.
    }

    private func bubbleWidth(in layout: RowLayout) -> CGFloat {
        min(RowLayout.speechBubbleMaxWidth, layout.contentSize.width - 8)
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

    @State private var hasAppeared = false

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
            if card.isPrimary {
                SessionBadge(
                    text: model.isExpanded ? card.label : model.collapsedLabel,
                    isDimmed: model.focusedSession == nil,
                    isHighlighted: model.isExpanded,
                    hasAttentionDot: !model.isExpanded && model.hasHiddenAttention
                )
                .frame(height: RowLayout.sessionBadgeReservedHeight)
            } else {
                SessionBadge(text: card.label, isDimmed: false, isHighlighted: false, hasAttentionDot: false)
                    .frame(height: RowLayout.sessionBadgeReservedHeight)
            }
        }
        .frame(height: cardHeight, alignment: .bottom)
        .opacity(card.isPrimary ? 1 : (hasAppeared ? 0.92 : 0))
        .onAppear {
            if card.isPrimary { hasAppeared = true; return }
            withAnimation(.easeOut(duration: 0.18)) { hasAppeared = true }
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

            StatusBadge(state: state, isCompact: !card.isPrimary)
                .offset(x: card.isPrimary ? 10 : 6, y: card.isPrimary ? -8 : -6)
                .animation(.spring(duration: 0.25), value: state)

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
            .multilineTextAlignment(.center)
            .lineLimit(2)
            .fixedSize(horizontal: false, vertical: true)
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

// MARK: - Session badge

struct SessionBadge: View {
    let text: String
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
        }
        .padding(.horizontal, 7)
        .padding(.vertical, 2.5)
        .background(Capsule().fill(Color(nsColor: .windowBackgroundColor).opacity(0.85)))
        .overlay(Capsule().strokeBorder(isHighlighted ? Color.accentColor.opacity(0.7) : Color.primary.opacity(0.1), lineWidth: isHighlighted ? 1.2 : 1))
        .frame(maxWidth: 200)
    }
}
