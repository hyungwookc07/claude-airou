import SwiftUI

/// The whole overlay content: speech bubble on top, sprite in the middle, session badge below.
struct PetView: View {
    @ObservedObject var model: PetViewModel

    var body: some View {
        VStack(spacing: 4) {
            speechBubbleArea
                .frame(height: PetViewModel.speechBubbleReservedHeight, alignment: .bottom)

            spriteWithStatus
                .frame(width: model.spriteSize.width, height: model.spriteSize.height)

            SessionBadge(text: model.sessionBadgeText, isDimmed: model.focusedSession == nil)
                .frame(height: PetViewModel.sessionBadgeReservedHeight)
        }
        .padding(.horizontal, PetViewModel.horizontalPadding)
        .frame(width: model.contentSize.width, height: model.contentSize.height)
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

    // MARK: - Sprite

    private var spriteWithStatus: some View {
        ZStack(alignment: .topTrailing) {
            SpriteCanvas(frame: model.currentFrame, palette: model.palette, pixelScale: model.pixelScale)
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

            StatusBadge(state: model.displayState)
                .offset(x: 10, y: -8)
                .animation(.spring(duration: 0.25), value: model.displayState)

            FloatingHeart(trigger: model.petReactionTrigger)
                .frame(maxWidth: .infinity, alignment: .center)
                .offset(y: -6)
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
            .frame(maxWidth: 210)
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

    var body: some View {
        switch state {
        case .waitingApproval:
            BadgeIcon(systemName: "clock.fill", tint: .red, isPulsing: true, isSpinning: false)
        case .needsInput:
            BadgeIcon(systemName: "questionmark.circle.fill", tint: .orange, isPulsing: true, isSpinning: false)
        case .done:
            BadgeIcon(systemName: "checkmark.circle.fill", tint: .green, isPulsing: false, isSpinning: false)
        case .error:
            BadgeIcon(systemName: "exclamationmark.triangle.fill", tint: .red, isPulsing: false, isSpinning: false)
        case .working:
            BadgeIcon(systemName: "gearshape.fill", tint: .blue, isPulsing: false, isSpinning: true)
        case .thinking:
            ThinkingDots()
        case .hello:
            BadgeIcon(systemName: "hand.wave.fill", tint: .yellow, isPulsing: false, isSpinning: false)
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

    @State private var isRotating = false

    var body: some View {
        Image(systemName: systemName)
            .font(.system(size: 13, weight: .bold))
            .foregroundStyle(.white, tint)
            .symbolRenderingMode(.palette)
            .padding(3)
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

struct ThinkingDots: View {
    @State private var phase = 0

    private let timer = Timer.publish(every: 0.35, on: .main, in: .common).autoconnect()

    var body: some View {
        HStack(spacing: 2) {
            ForEach(0..<3, id: \.self) { index in
                Circle()
                    .fill(Color.primary.opacity(index == phase ? 0.9 : 0.35))
                    .frame(width: 4, height: 4)
            }
        }
        .padding(.horizontal, 6)
        .padding(.vertical, 5)
        .background(Capsule().fill(Color(nsColor: .windowBackgroundColor)))
        .overlay(Capsule().strokeBorder(Color.primary.opacity(0.12)))
        .shadow(color: .black.opacity(0.18), radius: 2, y: 1)
        .onReceive(timer) { _ in phase = (phase + 1) % 3 }
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

    var body: some View {
        Text(text)
            .font(.system(size: 9.5, weight: .semibold, design: .rounded))
            .foregroundStyle(isDimmed ? .tertiary : .secondary)
            .lineLimit(1)
            .padding(.horizontal, 7)
            .padding(.vertical, 2.5)
            .background(Capsule().fill(Color(nsColor: .windowBackgroundColor).opacity(0.85)))
            .overlay(Capsule().strokeBorder(Color.primary.opacity(0.1)))
            .frame(maxWidth: 200)
    }
}
