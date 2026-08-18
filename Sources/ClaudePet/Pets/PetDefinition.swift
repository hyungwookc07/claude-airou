import Foundation

/// A pet is a JSON "sprite pack": a palette plus pixel-art frames per state.
///
/// ```json
/// {
///   "id": "mochi-cat", "name": "Mochi", "species": "cat",
///   "fps": 3,
///   "palette": { "k": "#2b2b2b", "o": "#f4a24a" },
///   "phrases": { "pet": ["Purr…"] },
///   "frames": { "idle": [["..kk..", ".kook."], ["..kk..", ".kook."]] }
/// }
/// ```
/// Every frame is an array of equally long strings; each character is a palette key,
/// `.` or space for transparent. All frames of a pet must share one grid size.
struct PetDefinition: Codable, Equatable {
    static let transparentCharacters: Set<Character> = [".", " "]
    static let defaultFramesPerSecond: Double = 3
    static let minimumGridSide = 4
    static let maximumGridSide = 64

    var id: String
    var name: String
    var species: String
    var description: String?
    var author: String?
    var fps: Double?
    var palette: [String: String]
    var phrases: [String: [String]]?
    var frames: [String: [[String]]]

    var framesPerSecond: Double {
        guard let fps, fps > 0 else { return Self.defaultFramesPerSecond }
        return min(max(fps, 0.5), 12)
    }

    var petPhrases: [String] {
        let list = phrases?["pet"] ?? []
        return list.isEmpty ? ["♥"] : list
    }

    /// Grid size derived from the idle frames (validated to be uniform).
    var gridSize: (width: Int, height: Int) {
        guard let firstFrame = frames(for: .idle).first, let firstRow = firstFrame.first else {
            return (0, 0)
        }
        return (firstRow.count, firstFrame.count)
    }

    /// Frames for a state, following the fallback chain when the pet does not define it.
    func frames(for state: PetState) -> [[String]] {
        if let direct = frames[state.rawValue], !direct.isEmpty { return direct }
        for fallback in state.fallbackStates {
            if let candidate = frames[fallback.rawValue], !candidate.isEmpty { return candidate }
        }
        return frames["idle"] ?? []
    }

    // MARK: - Validation

    struct ValidationError: LocalizedError {
        let problems: [String]
        var errorDescription: String? { problems.joined(separator: "\n") }
    }

    /// Returns non-fatal warnings; throws `ValidationError` for problems that make the pet unusable.
    @discardableResult
    func validate() throws -> [String] {
        var problems: [String] = []
        var warnings: [String] = []

        if id.trimmingCharacters(in: .whitespaces).isEmpty { problems.append("`id` must not be empty") }
        if id.contains(where: { !($0.isLetter || $0.isNumber || $0 == "-" || $0 == "_") }) {
            problems.append("`id` may only contain letters, digits, '-' and '_' (got \"\(id)\")")
        }
        if name.trimmingCharacters(in: .whitespaces).isEmpty { problems.append("`name` must not be empty") }

        var paletteByCharacter: [Character: String] = [:]
        for (key, hex) in palette {
            guard key.count == 1, let character = key.first else {
                problems.append("palette key \"\(key)\" must be exactly one character")
                continue
            }
            if Self.transparentCharacters.contains(character) {
                problems.append("palette key \"\(key)\" is reserved for transparency")
            }
            if PixelColor(hex: hex) == nil {
                problems.append("palette[\"\(key)\"] = \"\(hex)\" is not a #RRGGBB / #RRGGBBAA color")
            }
            paletteByCharacter[character] = hex
        }

        guard let idleFrames = frames["idle"], !idleFrames.isEmpty else {
            throw ValidationError(problems: problems + ["`frames.idle` is required and must contain at least one frame"])
        }
        guard let referenceRow = idleFrames[0].first else {
            throw ValidationError(problems: problems + ["`frames.idle[0]` has no rows"])
        }
        let expectedWidth = referenceRow.count
        let expectedHeight = idleFrames[0].count
        if expectedWidth < Self.minimumGridSide || expectedHeight < Self.minimumGridSide {
            problems.append("grid must be at least \(Self.minimumGridSide)x\(Self.minimumGridSide) (got \(expectedWidth)x\(expectedHeight))")
        }
        if expectedWidth > Self.maximumGridSide || expectedHeight > Self.maximumGridSide {
            problems.append("grid must be at most \(Self.maximumGridSide)x\(Self.maximumGridSide) (got \(expectedWidth)x\(expectedHeight))")
        }

        var usedCharacters: Set<Character> = []
        for (stateKey, stateFrames) in frames {
            if PetState(rawValue: stateKey) == nil {
                warnings.append("frames.\(stateKey): unknown state, ignored (known: \(PetState.allCases.map(\.rawValue).joined(separator: ", ")))")
                continue
            }
            if stateFrames.isEmpty {
                warnings.append("frames.\(stateKey): empty, will fall back")
                continue
            }
            for (frameIndex, frame) in stateFrames.enumerated() {
                if frame.count != expectedHeight {
                    problems.append("frames.\(stateKey)[\(frameIndex)] has \(frame.count) rows, expected \(expectedHeight)")
                }
                for (rowIndex, row) in frame.enumerated() {
                    if row.count != expectedWidth {
                        problems.append("frames.\(stateKey)[\(frameIndex)] row \(rowIndex) has \(row.count) columns, expected \(expectedWidth)")
                    }
                    for character in row where !Self.transparentCharacters.contains(character) {
                        usedCharacters.insert(character)
                        if paletteByCharacter[character] == nil {
                            problems.append("frames.\(stateKey)[\(frameIndex)] row \(rowIndex) uses \"\(character)\" which is not in the palette")
                        }
                    }
                }
            }
        }

        for (character, _) in paletteByCharacter where !usedCharacters.contains(character) {
            warnings.append("palette key \"\(character)\" is never used")
        }
        for state in PetState.allCases where frames[state.rawValue] == nil || frames[state.rawValue]?.isEmpty == true {
            if state != .idle {
                warnings.append("no frames for \(state.rawValue); falling back to \(state.fallbackStates.first?.rawValue ?? "idle")")
            }
        }

        // Duplicate problems are noisy when a whole frame is the wrong width; keep unique, ordered.
        var seen: Set<String> = []
        let uniqueProblems = problems.filter { seen.insert($0).inserted }
        if !uniqueProblems.isEmpty {
            throw ValidationError(problems: uniqueProblems)
        }
        return warnings
    }

    // MARK: - Loading

    static func load(from url: URL) throws -> PetDefinition {
        let data = try Data(contentsOf: url)
        return try decode(data)
    }

    static func decode(_ data: Data) throws -> PetDefinition {
        try JSONDecoder().decode(PetDefinition.self, from: data)
    }
}

/// A palette color parsed from `#RRGGBB` or `#RRGGBBAA`. Components are 0...1.
struct PixelColor: Equatable, Hashable {
    let red: Double
    let green: Double
    let blue: Double
    let alpha: Double

    init?(hex: String) {
        var text = hex.trimmingCharacters(in: .whitespaces)
        if text.hasPrefix("#") { text.removeFirst() }
        // UInt64(_:radix:) tolerates a leading sign; require pure hex digits.
        guard text.count == 6 || text.count == 8,
              text.allSatisfy(\.isHexDigit),
              let value = UInt64(text, radix: 16) else { return nil }
        if text.count == 6 {
            red = Double((value >> 16) & 0xFF) / 255
            green = Double((value >> 8) & 0xFF) / 255
            blue = Double(value & 0xFF) / 255
            alpha = 1
        } else {
            red = Double((value >> 24) & 0xFF) / 255
            green = Double((value >> 16) & 0xFF) / 255
            blue = Double((value >> 8) & 0xFF) / 255
            alpha = Double(value & 0xFF) / 255
        }
    }
}

/// Palette resolved to characters, ready for rendering.
struct ResolvedPalette {
    let colorsByCharacter: [Character: PixelColor]

    init(definition: PetDefinition) {
        var map: [Character: PixelColor] = [:]
        for (key, hex) in definition.palette {
            guard let character = key.first, key.count == 1, let color = PixelColor(hex: hex) else { continue }
            map[character] = color
        }
        colorsByCharacter = map
    }

    func color(for character: Character) -> PixelColor? {
        colorsByCharacter[character]
    }
}
