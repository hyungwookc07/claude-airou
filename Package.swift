// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "claude-airou",
    platforms: [.macOS(.v14)],
    targets: [
        .executableTarget(
            name: "claude-airou",
            path: "Sources/ClaudeAirou",
            resources: [
                .embedInCode("Resources/pets/airou-felyne.json"),
                .embedInCode("Resources/pets/mochi-cat.json"),
                .embedInCode("Resources/pets/quackers-duck.json"),
                .embedInCode("Resources/pets/boo-ghost.json"),
                .embedInCode("Resources/pets/jelly-slime.json"),
                .embedInCode("Resources/pets/bolt-robot.json"),
                .embedInCode("Resources/pets/inky-octopus.json"),
                .embedInCode("Resources/pets/clawd-claude.json"),
            ]
        )
    ]
)
