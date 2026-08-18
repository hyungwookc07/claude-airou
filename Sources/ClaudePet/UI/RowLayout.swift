import CoreGraphics
import Foundation

/// Deterministic horizontal layout of one or more session "cards" (sprite + label).
/// Both the SwiftUI view and the AppKit panel derive geometry from this, so the panel can
/// grow/shrink while keeping the primary pet exactly where it was on screen.
struct RowLayout: Equatable {
    struct Card: Equatable, Identifiable {
        let sessionId: String?     // nil when there is no session at all
        let isPrimary: Bool
        let pixelScale: CGFloat    // integer-valued so pixels stay crisp
        let width: CGFloat
        let x: CGFloat             // leading edge within the content
        let label: String

        var id: String { sessionId ?? "none" }
        var centerX: CGFloat { x + width / 2 }
    }

    static let minimumContentWidth: CGFloat = 220
    static let horizontalPadding: CGFloat = 16
    static let cardSpacing: CGFloat = 10
    static let minimumCardWidth: CGFloat = 72
    static let maximumCardWidth: CGFloat = 132
    static let sideSpriteScaleFactor: CGFloat = 0.7
    static let speechBubbleReservedHeight: CGFloat = 58
    static let sessionBadgeReservedHeight: CGFloat = 22
    static let verticalPadding: CGFloat = 12
    /// Rough width of one label character at the badge font, for card sizing without text measurement.
    static let approximateLabelCharacterWidth: CGFloat = 6.2
    static let labelHorizontalInset: CGFloat = 18

    let cards: [Card]
    let contentSize: CGSize
    let primarySpriteSize: CGSize

    var primaryCard: Card { cards.first(where: \.isPrimary) ?? cards[0] }
    var primaryCenterX: CGFloat { primaryCard.centerX }

    static func sideScale(for pixelScale: CGFloat) -> CGFloat {
        max(2, (pixelScale * sideSpriteScaleFactor).rounded(.down))
    }

    /// - Parameters:
    ///   - labels: one per card, left to right; `primaryIndex` marks the full-size card.
    static func make(
        gridSize: (width: Int, height: Int),
        pixelScale: CGFloat,
        labels: [String],
        sessionIds: [String?],
        primaryIndex: Int
    ) -> RowLayout {
        precondition(labels.count == sessionIds.count && !labels.isEmpty)
        let sideScale = sideScale(for: pixelScale)
        let primarySpriteSize = CGSize(width: CGFloat(gridSize.width) * pixelScale, height: CGFloat(gridSize.height) * pixelScale)

        var cardWidths: [CGFloat] = []
        for (index, label) in labels.enumerated() {
            let scale = index == primaryIndex ? pixelScale : sideScale
            let spriteWidth = CGFloat(gridSize.width) * scale
            let labelWidth = CGFloat(label.count) * approximateLabelCharacterWidth + labelHorizontalInset
            let width = min(maximumCardWidth, max(minimumCardWidth, spriteWidth, labelWidth))
            cardWidths.append(width.rounded())
        }

        let rowWidth = cardWidths.reduce(0, +) + cardSpacing * CGFloat(max(0, cardWidths.count - 1))
        let contentWidth = max(minimumContentWidth, rowWidth + horizontalPadding * 2)
        let rowLeading = ((contentWidth - rowWidth) / 2).rounded()
        let contentHeight = speechBubbleReservedHeight + primarySpriteSize.height + sessionBadgeReservedHeight + verticalPadding

        var cards: [Card] = []
        var x = rowLeading
        for index in labels.indices {
            let isPrimary = index == primaryIndex
            cards.append(Card(
                sessionId: sessionIds[index],
                isPrimary: isPrimary,
                pixelScale: isPrimary ? pixelScale : sideScale,
                width: cardWidths[index],
                x: x,
                label: labels[index]
            ))
            x += cardWidths[index] + cardSpacing
        }

        return RowLayout(
            cards: cards,
            contentSize: CGSize(width: contentWidth, height: contentHeight),
            primarySpriteSize: primarySpriteSize
        )
    }

    /// The card under a content-space x coordinate, if any.
    func card(atContentX x: CGFloat) -> Card? {
        cards.first { x >= $0.x && x <= $0.x + $0.width }
    }
}
