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
    static let speechBubbleReservedHeight: CGFloat = 66
    /// Gap between the bubble's tail and the top of the sprite.
    static let speechBubbleBottomInset: CGFloat = 12
    static let sessionBadgeReservedHeight: CGFloat = 22
    /// Battery gauge row between the sprite and the label (0 when the gauge is off).
    static let gaugeReservedHeight: CGFloat = 16
    static let verticalPadding: CGFloat = 12
    /// Rough width of one label character at the badge font, for card sizing without text measurement.
    static let approximateLabelCharacterWidth: CGFloat = 6.2
    static let labelHorizontalInset: CGFloat = 18
    /// Room for the status icon that sits at the right end of the label capsule.
    static let labelStatusIconAllowance: CGFloat = 18
    /// The speech bubble is centred over the primary card; the row reserves room for the current
    /// bubble width around that centre so the bubble never has to be pushed off the pet.
    static let speechBubbleMaxWidth: CGFloat = 300
    static let speechBubbleEdgeMargin: CGFloat = 4

    let cards: [Card]
    let contentSize: CGSize
    let primarySpriteSize: CGSize
    /// Width the speech bubble will take for the current text (0 when there is no bubble).
    let speechBubbleWidth: CGFloat
    /// Whether a gauge row is laid out under the sprites.
    let showsGauge: Bool

    var gaugeHeight: CGFloat { showsGauge ? Self.gaugeReservedHeight : 0 }
    /// Height of one card: sprite + gauge + label (+ spacing).
    var cardHeight: CGFloat { primarySpriteSize.height + gaugeHeight + Self.sessionBadgeReservedHeight + 4 }

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
        primaryIndex: Int,
        speechBubbleWidth: CGFloat = 0,
        showsGauge: Bool = false
    ) -> RowLayout {
        precondition(labels.count == sessionIds.count && !labels.isEmpty)
        let sideScale = sideScale(for: pixelScale)
        let primarySpriteSize = CGSize(width: CGFloat(gridSize.width) * pixelScale, height: CGFloat(gridSize.height) * pixelScale)

        var cardWidths: [CGFloat] = []
        for (index, label) in labels.enumerated() {
            let scale = index == primaryIndex ? pixelScale : sideScale
            let spriteWidth = CGFloat(gridSize.width) * scale
            let labelWidth = CGFloat(label.count) * approximateLabelCharacterWidth + labelHorizontalInset + labelStatusIconAllowance
            let width = min(maximumCardWidth, max(minimumCardWidth, spriteWidth, labelWidth))
            cardWidths.append((width / 2).rounded(.up) * 2) // even, so card centres are whole points
        }

        let rowWidth = cardWidths.reduce(0, +) + cardSpacing * CGFloat(max(0, cardWidths.count - 1))
        var contentWidth = max(minimumContentWidth, rowWidth + horizontalPadding * 2)
        var rowLeading = ((contentWidth - rowWidth) / 2).rounded()

        // Make room for the bubble around the primary card's centre (it may sit off-centre in the row).
        let primaryLeading = rowLeading + cardWidths[..<primaryIndex].reduce(0, +) + cardSpacing * CGFloat(primaryIndex)
        let primaryCenter = primaryLeading + cardWidths[primaryIndex] / 2
        let bubbleWidth = min(speechBubbleMaxWidth, max(0, speechBubbleWidth))
        let bubbleHalf = bubbleWidth / 2 + speechBubbleEdgeMargin
        let leftShortfall = max(0, bubbleHalf - primaryCenter)
        rowLeading += leftShortfall
        contentWidth += leftShortfall
        let rightShortfall = max(0, (primaryCenter + leftShortfall + bubbleHalf) - contentWidth)
        contentWidth += rightShortfall

        let gaugeHeight = showsGauge ? gaugeReservedHeight : 0
        let contentHeight = speechBubbleReservedHeight + primarySpriteSize.height + gaugeHeight + sessionBadgeReservedHeight + verticalPadding

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
            primarySpriteSize: primarySpriteSize,
            speechBubbleWidth: bubbleWidth,
            showsGauge: showsGauge
        )
    }

    /// The card under a content-space x coordinate, if any.
    func card(atContentX x: CGFloat) -> Card? {
        cards.first { x >= $0.x && x <= $0.x + $0.width }
    }
}
