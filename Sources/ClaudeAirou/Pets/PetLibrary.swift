import Foundation

/// Built-in pets are embedded in the binary at build time (SwiftPM `embedInCode`),
/// so the executable stays self-contained wherever it is copied.
enum BuiltInPets {
    /// Order here is the order in the menu.
    static let embeddedJSONFiles: [(fileName: String, bytes: [UInt8])] = [
        ("airou-felyne.json", PackageResources.airou_felyne_json),   // the mascot comes first (default for new installs)
        ("mochi-cat.json", PackageResources.mochi_cat_json),
        ("quackers-duck.json", PackageResources.quackers_duck_json),
        ("boo-ghost.json", PackageResources.boo_ghost_json),
        ("jelly-slime.json", PackageResources.jelly_slime_json),
        ("bolt-robot.json", PackageResources.bolt_robot_json),
        ("inky-octopus.json", PackageResources.inky_octopus_json),
        ("clawd-claude.json", PackageResources.clawd_claude_json),
    ]

    static func loadAll() -> [PetDefinition] {
        embeddedJSONFiles.compactMap { entry in
            do {
                let definition = try PetDefinition.decode(Data(entry.bytes))
                try definition.validate()
                return definition
            } catch {
                StandardError.print("claude-airou: built-in pet \(entry.fileName) is invalid: \(error.localizedDescription)")
                return nil
            }
        }
    }
}

/// Every pet available to the overlay: built-ins first, then `~/.claude-airou/pets/*.json`.
/// A user pet with the same id as a built-in overrides it.
struct PetLibrary {
    struct LoadedPet: Equatable {
        let definition: PetDefinition
        let sourceURL: URL?  // nil for built-ins
        var isBuiltIn: Bool { sourceURL == nil }
    }

    private(set) var pets: [LoadedPet] = []
    private(set) var loadProblems: [String] = []

    static func load(userPetsDirectory: URL = AppPaths.petsDirectory) -> PetLibrary {
        var library = PetLibrary()
        var petsById: [String: LoadedPet] = [:]
        var order: [String] = []

        for definition in BuiltInPets.loadAll() {
            petsById[definition.id] = LoadedPet(definition: definition, sourceURL: nil)
            order.append(definition.id)
        }

        if let entries = try? FileManager.default.contentsOfDirectory(
            at: userPetsDirectory,
            includingPropertiesForKeys: nil,
            options: [.skipsHiddenFiles]
        ) {
            for fileURL in entries.sorted(by: { $0.lastPathComponent < $1.lastPathComponent })
            where fileURL.pathExtension.lowercased() == "json" {
                do {
                    let definition = try PetDefinition.load(from: fileURL)
                    try definition.validate()
                    if petsById[definition.id] == nil { order.append(definition.id) }
                    petsById[definition.id] = LoadedPet(definition: definition, sourceURL: fileURL)
                } catch {
                    library.loadProblems.append("\(fileURL.lastPathComponent): \(error.localizedDescription)")
                }
            }
        }

        library.pets = order.compactMap { petsById[$0] }
        return library
    }

    func pet(withId id: String?) -> LoadedPet? {
        guard let id else { return nil }
        return pets.first { $0.definition.id == id }
    }

    /// The pet to show: the configured one if it still exists, else the first built-in.
    func resolveSelectedPet(preferredId: String?) -> LoadedPet? {
        pet(withId: preferredId) ?? pets.first
    }
}
