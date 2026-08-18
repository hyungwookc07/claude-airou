import Foundation

/// User preferences persisted at `~/.claude-pet/config.json`.
struct AppConfig: Codable, Equatable {
    static let defaultPixelScale: Double = 5
    static let minimumPixelScale: Double = 1
    static let maximumPixelScale: Double = 12
    static let availablePixelScales: [(label: String, scale: Double)] = [
        ("Small", 3),
        ("Medium", 5),
        ("Large", 7),
    ]

    var selectedPetId: String?
    var pixelScale: Double = defaultPixelScale
    var windowOriginX: Double?
    var windowOriginY: Double?
    var isSpeechBubbleHidden: Bool = false
    var isClickThrough: Bool = false
    var isPetHidden: Bool = false

    init() {}

    // Tolerate partially written / older config files: every key is optional on decode.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        selectedPetId = try container.decodeIfPresent(String.self, forKey: .selectedPetId)
        let decodedScale = try container.decodeIfPresent(Double.self, forKey: .pixelScale) ?? Self.defaultPixelScale
        pixelScale = decodedScale.isFinite ? min(max(decodedScale, Self.minimumPixelScale), Self.maximumPixelScale) : Self.defaultPixelScale
        windowOriginX = try container.decodeIfPresent(Double.self, forKey: .windowOriginX)
        windowOriginY = try container.decodeIfPresent(Double.self, forKey: .windowOriginY)
        isSpeechBubbleHidden = try container.decodeIfPresent(Bool.self, forKey: .isSpeechBubbleHidden) ?? false
        isClickThrough = try container.decodeIfPresent(Bool.self, forKey: .isClickThrough) ?? false
        isPetHidden = try container.decodeIfPresent(Bool.self, forKey: .isPetHidden) ?? false
    }

    static func load(from url: URL = AppPaths.configFile) -> AppConfig {
        guard let data = try? Data(contentsOf: url),
              let config = try? JSONDecoder().decode(AppConfig.self, from: data) else {
            return AppConfig()
        }
        return config
    }

    func save(to url: URL = AppPaths.configFile) {
        do {
            try AppPaths.ensureDirectoryExists(url.deletingLastPathComponent())
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            try encoder.encode(self).write(to: url, options: .atomic)
        } catch {
            StandardError.print("claude-pet: could not save config: \(error.localizedDescription)")
        }
    }
}

/// Small helper so nothing in the hook path ever writes to stdout by accident
/// (Claude Code feeds hook stdout of some events back into the model context).
enum StandardError {
    static func print(_ message: String) {
        FileHandle.standardError.write(Data((message + "\n").utf8))
    }
}
