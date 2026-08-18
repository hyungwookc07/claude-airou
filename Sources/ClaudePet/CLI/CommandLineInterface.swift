import Foundation

/// Tiny hand-rolled CLI so the binary has zero dependencies.
enum CommandLineInterface {
    static let version = "0.1.0"

    static let usageText = """
    claude-pet — a Codex-style desktop pet for Claude Code (macOS)

    USAGE
      claude-pet                       Run the overlay (menu bar icon + floating pet)
      claude-pet run                   Same as above
      claude-pet hook                  Claude Code hook entry point (reads hook JSON on stdin)
      claude-pet install-hooks [--print] [--settings PATH]
                                       Merge hook entries into ~/.claude/settings.json (backup first)
                                       --print only prints the JSON snippet, changes nothing
      claude-pet uninstall-hooks [--settings PATH]
      claude-pet simulate STATE [--message TEXT] [--session ID] [--cwd PATH]
                                       Write a fake session so you can see the pet react
                                       STATE: \(PetState.allCases.map(\.rawValue).joined(separator: " | ")) | clear | demo
      claude-pet pets                  List available pets (built-in + ~/.claude-pet/pets)
      claude-pet validate FILE.json    Validate a pet JSON file
      claude-pet render PET_ID|FILE [--out DIR] [--scale N] [--bg #RRGGBB]
                                       Render every frame to PNG (+ sheet.png) to eyeball pixel art
      claude-pet preview PET_ID|FILE [--state STATE] [--solid]
                                       Print frames as ASCII
      claude-pet status                Print the sessions the overlay currently sees
      claude-pet snapshot [--out FILE.png]
                                       Ask the running overlay to save a PNG of itself
      claude-pet click [primary|X]     Click the running overlay (primary pet, or x in points) — for testing
      claude-pet help

    FILES
      ~/.claude-pet/config.json        preferences (pet, size, position)
      ~/.claude-pet/pets/*.json        your custom pets (see skills/hatch-pet)
      ~/.claude-pet/state/*.json       live session state written by the hook
      ~/.claude-pet/hook.log           what the hook saw (auto-truncated)
    """

    struct ParsedArguments {
        var positional: [String] = []
        var options: [String: String] = [:]
        var flags: Set<String> = []

        func option(_ name: String) -> String? { options[name] }
        func hasFlag(_ name: String) -> Bool { flags.contains(name) }
    }

    /// Options that never take a value. Everything else written as `--name value` is an option.
    static let booleanFlagNames: Set<String> = ["solid", "print", "help", "version", "h"]

    /// `--key value` becomes an option, `--flag` becomes a flag; `--key=value` is also accepted.
    static func parse(_ arguments: [String]) -> ParsedArguments {
        var parsed = ParsedArguments()
        var index = 0
        while index < arguments.count {
            let argument = arguments[index]
            if argument == "-h" {
                parsed.flags.insert("help")
            } else if argument.hasPrefix("--") {
                let name = String(argument.dropFirst(2))
                if let equalsIndex = name.firstIndex(of: "=") {
                    parsed.options[String(name[..<equalsIndex])] = String(name[name.index(after: equalsIndex)...])
                } else if booleanFlagNames.contains(name) {
                    parsed.flags.insert(name)
                } else if index + 1 < arguments.count, !arguments[index + 1].hasPrefix("--") {
                    parsed.options[name] = arguments[index + 1]
                    index += 1
                } else {
                    parsed.flags.insert(name)
                }
            } else {
                parsed.positional.append(argument)
            }
            index += 1
        }
        return parsed
    }

    // MARK: - Dispatch

    /// Returns an exit code, or `nil` when the overlay app should be started (it never returns).
    static func dispatch(arguments: [String]) -> Int32? {
        let parsed = parse(arguments)
        if parsed.hasFlag("help") || parsed.hasFlag("h") {
            print(usageText)
            return 0
        }
        if parsed.hasFlag("version") {
            print("claude-pet \(version)")
            return 0
        }
        let command = parsed.positional.first ?? "run"
        let rest = Array(parsed.positional.dropFirst())

        switch command {
        case "run":
            return nil
        case "hook":
            return HookCommand.run()
        case "install-hooks":
            return runInstallHooks(parsed)
        case "uninstall-hooks":
            return runUninstallHooks(parsed)
        case "simulate":
            return runSimulate(rest, parsed)
        case "pets", "list":
            return runListPets()
        case "validate":
            return runValidate(rest)
        case "render":
            return runRender(rest, parsed)
        case "preview":
            return runPreview(rest, parsed)
        case "status", "sessions":
            return runStatus()
        case "snapshot":
            return runSnapshot(parsed)
        case "click":
            return runClick(rest)
        case "help":
            print(usageText)
            return 0
        case "version":
            print("claude-pet \(version)")
            return 0
        default:
            StandardError.print("claude-pet: unknown command \"\(command)\"\n")
            StandardError.print(usageText)
            return 2
        }
    }

    // MARK: - Commands

    private static func settingsURL(from parsed: ParsedArguments) -> URL {
        if let path = parsed.option("settings") {
            return URL(fileURLWithPath: (path as NSString).expandingTildeInPath)
        }
        return AppPaths.claudeSettingsFile
    }

    private static func runInstallHooks(_ parsed: ParsedArguments) -> Int32 {
        let installer = HooksInstaller(settingsURL: settingsURL(from: parsed))
        if parsed.hasFlag("print") {
            print(installer.snippetJSON())
            return 0
        }
        do {
            let report = try installer.install()
            print("Claude Code hooks installed.")
            print(report.summaryText)
            print("\nRestart running Claude Code sessions (or start a new one) for hooks to take effect.")
            return 0
        } catch {
            StandardError.print("claude-pet: install failed: \(error.localizedDescription)")
            return 1
        }
    }

    private static func runUninstallHooks(_ parsed: ParsedArguments) -> Int32 {
        let installer = HooksInstaller(settingsURL: settingsURL(from: parsed))
        do {
            let report = try installer.uninstall()
            print("Claude Code hooks removed.")
            print(report.summaryText)
            return 0
        } catch {
            StandardError.print("claude-pet: uninstall failed: \(error.localizedDescription)")
            return 1
        }
    }

    private static func runSimulate(_ positional: [String], _ parsed: ParsedArguments) -> Int32 {
        guard let stateText = positional.first else {
            StandardError.print("claude-pet simulate: missing STATE (\(PetState.allCases.map(\.rawValue).joined(separator: " | ")) | clear | demo)")
            return 2
        }
        let store = SessionStateStore()
        let sessionId = parsed.option("session") ?? "simulated"
        let cwd = parsed.option("cwd") ?? FileManager.default.currentDirectoryPath

        if stateText == "clear" {
            store.remove(sessionId: sessionId)
            print("Removed simulated session \"\(sessionId)\".")
            return 0
        }

        func write(_ state: PetState, _ message: String) {
            let snapshot = SessionSnapshot(
                sessionId: sessionId,
                cwd: cwd,
                state: state,
                message: message,
                lastEventName: "simulate",
                toolName: nil,
                updatedAtEpochSeconds: Date().timeIntervalSince1970
            )
            do {
                try store.write(snapshot)
                print("\(state.rawValue): \(message)")
            } catch {
                StandardError.print("claude-pet simulate: \(error.localizedDescription)")
            }
        }

        if stateText == "demo" {
            let script: [(PetState, String, TimeInterval)] = [
                (.hello, "Hi! Ready when you are", 3),
                (.thinking, "Thinking…", 3),
                (.working, "Reading main.swift", 3),
                (.working, "Running: swift build", 3),
                (.waitingApproval, "Approve? Running: git push", 5),
                (.working, "Editing README.md", 3),
                (.error, "Bash failed — recovering…", 3),
                (.thinking, "Thinking…", 2),
                (.done, "Done!", 4),
                (.needsInput, "Waiting for you…", 4),
            ]
            print("Demo: cycling through states (Ctrl-C to stop). Session \"\(sessionId)\".")
            for (state, message, seconds) in script {
                write(state, message)
                Thread.sleep(forTimeInterval: seconds)
            }
            store.remove(sessionId: sessionId)
            print("Demo finished; simulated session removed.")
            return 0
        }

        guard let state = PetState.parse(stateText) else {
            StandardError.print("claude-pet simulate: unknown state \"\(stateText)\"")
            return 2
        }
        let message = parsed.option("message") ?? defaultMessage(for: state)
        write(state, message)
        return 0
    }

    private static func defaultMessage(for state: PetState) -> String {
        switch state {
        case .hello: return "Hi! Ready when you are"
        case .idle: return ""
        case .thinking: return "Thinking…"
        case .working: return "Reading a file"
        case .waitingApproval: return "Approve? Running: git push"
        case .needsInput: return "Waiting for you…"
        case .done: return "Done!"
        case .error: return "Something failed — recovering…"
        }
    }

    private static func runListPets() -> Int32 {
        let library = PetLibrary.load()
        for loadedPet in library.pets {
            let definition = loadedPet.definition
            let grid = definition.gridSize
            let origin = loadedPet.sourceURL?.path ?? "built-in"
            print("\(definition.id)\t\(definition.name) the \(definition.species)\t\(grid.width)x\(grid.height)\t\(origin)")
        }
        for problem in library.loadProblems {
            StandardError.print("skipped: \(problem)")
        }
        return 0
    }

    private static func loadPet(reference: String) throws -> PetDefinition {
        let expanded = (reference as NSString).expandingTildeInPath
        var isDirectory: ObjCBool = false
        if FileManager.default.fileExists(atPath: expanded, isDirectory: &isDirectory), !isDirectory.boolValue {
            return try PetDefinition.load(from: URL(fileURLWithPath: expanded))
        }
        let library = PetLibrary.load()
        if let loadedPet = library.pet(withId: reference) {
            return loadedPet.definition
        }
        throw HooksInstaller.InstallError(message: "no pet with id \"\(reference)\" and no such file")
    }

    private static func runValidate(_ positional: [String]) -> Int32 {
        guard let path = positional.first else {
            StandardError.print("claude-pet validate: missing FILE.json")
            return 2
        }
        do {
            let definition = try PetDefinition.load(from: URL(fileURLWithPath: (path as NSString).expandingTildeInPath))
            let warnings = try definition.validate()
            let grid = definition.gridSize
            print("OK: \(definition.id) (\(definition.name) the \(definition.species)), grid \(grid.width)x\(grid.height), \(definition.frames.count) state(s)")
            for warning in warnings {
                print("warning: \(warning)")
            }
            return 0
        } catch let decodingError as DecodingError {
            StandardError.print("INVALID JSON STRUCTURE: \(describe(decodingError))")
            return 1
        } catch {
            StandardError.print("INVALID:\n\(error.localizedDescription)")
            return 1
        }
    }

    private static func describe(_ error: DecodingError) -> String {
        switch error {
        case let .keyNotFound(key, context):
            return "missing key \"\(key.stringValue)\" at \(context.codingPath.map(\.stringValue).joined(separator: "."))"
        case let .typeMismatch(type, context):
            return "wrong type at \(context.codingPath.map(\.stringValue).joined(separator: ".")) (expected \(type))"
        case let .valueNotFound(type, context):
            return "null value at \(context.codingPath.map(\.stringValue).joined(separator: ".")) (expected \(type))"
        case let .dataCorrupted(context):
            return "corrupted data: \(context.debugDescription)"
        @unknown default:
            return String(describing: error)
        }
    }

    private static func runRender(_ positional: [String], _ parsed: ParsedArguments) -> Int32 {
        guard let reference = positional.first else {
            StandardError.print("claude-pet render: missing PET_ID or FILE")
            return 2
        }
        do {
            let definition = try loadPet(reference: reference)
            try definition.validate()
            var scale = 8
            if let scaleText = parsed.option("scale") {
                guard let parsedScale = Int(scaleText), (1...64).contains(parsedScale) else {
                    StandardError.print("claude-pet render: --scale must be an integer between 1 and 64 (got \"\(scaleText)\")")
                    return 2
                }
                scale = parsedScale
            }
            var background: PixelColor?
            if let backgroundText = parsed.option("bg") {
                guard let parsedBackground = PixelColor(hex: backgroundText) else {
                    StandardError.print("claude-pet render: --bg must be #RRGGBB or #RRGGBBAA (got \"\(backgroundText)\")")
                    return 2
                }
                background = parsedBackground
            }
            let outputDirectory = URL(fileURLWithPath: ((parsed.option("out") ?? "./render-\(definition.id)") as NSString).expandingTildeInPath)
            let written = try SpriteRenderer.renderAll(pet: definition, outputDirectory: outputDirectory, pixelScale: scale, backgroundColor: background)
            print("Rendered \(written.count) file(s) to \(outputDirectory.path)")
            print("Contact sheet: \(outputDirectory.appendingPathComponent("sheet.png").path)")
            return 0
        } catch {
            StandardError.print("claude-pet render: \(error.localizedDescription)")
            return 1
        }
    }

    private static func runPreview(_ positional: [String], _ parsed: ParsedArguments) -> Int32 {
        guard let reference = positional.first else {
            StandardError.print("claude-pet preview: missing PET_ID or FILE")
            return 2
        }
        do {
            let definition = try loadPet(reference: reference)
            let states: [PetState]
            if let stateText = parsed.option("state") {
                guard let state = PetState.parse(stateText) else {
                    StandardError.print("claude-pet preview: unknown state \"\(stateText)\"")
                    return 2
                }
                states = [state]
            } else {
                states = PetState.allCases
            }
            let solid = parsed.hasFlag("solid")
            for state in states {
                let frames = definition.frames(for: state)
                for (index, frame) in frames.enumerated() {
                    print("== \(state.rawValue) [\(index)] ==")
                    print(SpriteRenderer.asciiArt(frame: frame, solid: solid))
                }
            }
            return 0
        } catch {
            StandardError.print("claude-pet preview: \(error.localizedDescription)")
            return 1
        }
    }

    /// Asks the running overlay to render itself to a PNG (works without screen-recording permission).
    private static func runSnapshot(_ parsed: ParsedArguments) -> Int32 {
        var outputPath = parsed.option("out").map { ($0 as NSString).expandingTildeInPath }
        if let path = outputPath {
            // `--out ~/Desktop` means "put snapshot.png in there", never "replace that directory".
            var isDirectory: ObjCBool = false
            if FileManager.default.fileExists(atPath: path, isDirectory: &isDirectory), isDirectory.boolValue {
                outputPath = (path as NSString).appendingPathComponent("claude-pet-snapshot.png")
            }
        }
        let imageURL = AppPaths.snapshotImageFile
        do {
            try AppPaths.ensureDirectoryExists(AppPaths.rootDirectory)
            try? FileManager.default.removeItem(at: imageURL)
            try Data().write(to: AppPaths.snapshotRequestFile)
        } catch {
            StandardError.print("claude-pet snapshot: \(error.localizedDescription)")
            return 1
        }
        let deadline = Date().addingTimeInterval(5)
        while Date() < deadline {
            if FileManager.default.fileExists(atPath: imageURL.path) {
                if let outputPath {
                    do {
                        let data = try Data(contentsOf: imageURL)
                        try data.write(to: URL(fileURLWithPath: outputPath), options: .atomic) // overwrites a file, never a directory
                        print(outputPath)
                    } catch {
                        StandardError.print("claude-pet snapshot: could not copy to \(outputPath): \(error.localizedDescription)")
                        return 1
                    }
                } else {
                    print(imageURL.path)
                }
                return 0
            }
            Thread.sleep(forTimeInterval: 0.1)
        }
        try? FileManager.default.removeItem(at: AppPaths.snapshotRequestFile)
        StandardError.print("claude-pet snapshot: no answer from the overlay — is `claude-pet run` running?")
        return 1
    }

    /// Scripted click on the running overlay: `claude-pet click primary` or `claude-pet click 42` (x in points).
    private static func runClick(_ positional: [String]) -> Int32 {
        let target = positional.first ?? "primary"
        do {
            try AppPaths.ensureDirectoryExists(AppPaths.rootDirectory)
            try Data(target.utf8).write(to: AppPaths.clickRequestFile, options: .atomic)
            return 0
        } catch {
            StandardError.print("claude-pet click: \(error.localizedDescription)")
            return 1
        }
    }

    private static func runStatus() -> Int32 {
        let store = SessionStateStore()
        let sessions = store.loadAll()
        if sessions.isEmpty {
            print("No sessions in \(store.directory.path)")
            return 0
        }
        for session in sessions {
            let age = Int(session.ageSeconds)
            print("\(session.sessionId)\t\(session.projectName)\t\(session.state.rawValue) → \(session.effectiveState.rawValue)\t\(age)s ago\t\(session.message)")
        }
        return 0
    }
}
