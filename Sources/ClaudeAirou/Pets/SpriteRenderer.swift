import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

/// Renders pet frames to PNG (for `claude-airou render`, used to eyeball pixel art while designing pets)
/// and to ASCII (for `claude-airou preview`). The live overlay draws with SwiftUI instead.
enum SpriteRenderer {
    struct RenderError: LocalizedError {
        let message: String
        var errorDescription: String? { message }
    }

    /// Draws one frame into a fresh bitmap context at `pixelScale` device pixels per sprite pixel.
    static func makeImage(frame: [String], palette: ResolvedPalette, pixelScale: Int, backgroundColor: PixelColor?) throws -> CGImage {
        let gridHeight = frame.count
        let gridWidth = frame.first?.count ?? 0
        guard gridWidth > 0, gridHeight > 0 else {
            throw RenderError(message: "frame is empty")
        }
        let width = gridWidth * pixelScale
        let height = gridHeight * pixelScale
        let colorSpace = CGColorSpace(name: CGColorSpace.sRGB)!
        guard let context = CGContext(
            data: nil,
            width: width,
            height: height,
            bitsPerComponent: 8,
            bytesPerRow: 0,
            space: colorSpace,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        ) else {
            throw RenderError(message: "could not create bitmap context")
        }

        if let backgroundColor {
            context.setFillColor(red: backgroundColor.red, green: backgroundColor.green, blue: backgroundColor.blue, alpha: backgroundColor.alpha)
            context.fill(CGRect(x: 0, y: 0, width: width, height: height))
        }

        for (rowIndex, row) in frame.enumerated() {
            for (columnIndex, character) in row.enumerated() {
                guard let color = palette.color(for: character) else { continue }
                context.setFillColor(red: color.red, green: color.green, blue: color.blue, alpha: color.alpha)
                // CoreGraphics origin is bottom-left; sprite rows are top-down.
                let rect = CGRect(
                    x: columnIndex * pixelScale,
                    y: (gridHeight - 1 - rowIndex) * pixelScale,
                    width: pixelScale,
                    height: pixelScale
                )
                context.fill(rect)
            }
        }

        guard let image = context.makeImage() else {
            throw RenderError(message: "could not create image")
        }
        return image
    }

    static func writePNG(_ image: CGImage, to url: URL) throws {
        guard let destination = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil) else {
            throw RenderError(message: "could not create PNG destination at \(url.path)")
        }
        CGImageDestinationAddImage(destination, image, nil)
        guard CGImageDestinationFinalize(destination) else {
            throw RenderError(message: "could not write PNG at \(url.path)")
        }
    }

    /// Writes `<state>_<index>.png` for every frame plus `sheet.png` (all states side by side, one row per state).
    /// Returns the written file URLs.
    @discardableResult
    static func renderAll(pet: PetDefinition, outputDirectory: URL, pixelScale: Int, backgroundColor: PixelColor?) throws -> [URL] {
        try AppPaths.ensureDirectoryExists(outputDirectory)
        let palette = ResolvedPalette(definition: pet)
        var written: [URL] = []

        var maxFrameCount = 0
        for state in PetState.allCases {
            let frames = pet.frames(for: state)
            maxFrameCount = max(maxFrameCount, frames.count)
            for (index, frame) in frames.enumerated() {
                let image = try makeImage(frame: frame, palette: palette, pixelScale: pixelScale, backgroundColor: backgroundColor)
                let url = outputDirectory.appendingPathComponent("\(state.rawValue)_\(index).png")
                try writePNG(image, to: url)
                written.append(url)
            }
        }

        // Contact sheet: rows = states (in PetState order), columns = frames.
        let grid = pet.gridSize
        let cellWidth = grid.width * pixelScale
        let cellHeight = grid.height * pixelScale
        let gutter = pixelScale * 2
        let sheetWidth = maxFrameCount * (cellWidth + gutter) + gutter
        let sheetHeight = PetState.allCases.count * (cellHeight + gutter) + gutter
        let colorSpace = CGColorSpace(name: CGColorSpace.sRGB)!
        guard let sheet = CGContext(
            data: nil, width: sheetWidth, height: sheetHeight,
            bitsPerComponent: 8, bytesPerRow: 0, space: colorSpace,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        ) else {
            throw RenderError(message: "could not create sheet context")
        }
        let sheetBackground = backgroundColor ?? PixelColor(hex: "#3a3f4b")!
        sheet.setFillColor(red: sheetBackground.red, green: sheetBackground.green, blue: sheetBackground.blue, alpha: 1)
        sheet.fill(CGRect(x: 0, y: 0, width: sheetWidth, height: sheetHeight))

        for (stateIndex, state) in PetState.allCases.enumerated() {
            let frames = pet.frames(for: state)
            for (frameIndex, frame) in frames.enumerated() {
                let image = try makeImage(frame: frame, palette: palette, pixelScale: pixelScale, backgroundColor: nil)
                let x = gutter + frameIndex * (cellWidth + gutter)
                let y = sheetHeight - gutter - (stateIndex + 1) * (cellHeight + gutter) + gutter
                sheet.draw(image, in: CGRect(x: x, y: y, width: cellWidth, height: cellHeight))
            }
        }
        guard let sheetImage = sheet.makeImage() else {
            throw RenderError(message: "could not create sheet image")
        }
        let sheetURL = outputDirectory.appendingPathComponent("sheet.png")
        try writePNG(sheetImage, to: sheetURL)
        written.append(sheetURL)
        return written
    }

    /// ASCII preview: transparent → space, everything else → the palette character (or `#` when `solid`).
    static func asciiArt(frame: [String], solid: Bool) -> String {
        frame.map { row in
            String(row.map { character -> Character in
                if PetDefinition.transparentCharacters.contains(character) { return " " }
                return solid ? "#" : character
            })
        }.joined(separator: "\n")
    }
}
