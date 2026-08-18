import AppKit
import Combine
import SwiftUI

/// Wires everything together for `claude-pet run`: config, pet library, view model, overlay panel, menu bar item.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSMenuDelegate {
    private var config = AppConfig.load()
    private var library = PetLibrary.load()
    private var viewModel: PetViewModel!
    private var panel: OverlayPanel!
    private var statusItem: NSStatusItem!
    private var cancellables: Set<AnyCancellable> = []

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)

        guard let selectedPet = library.resolveSelectedPet(preferredId: config.selectedPetId) else {
            StandardError.print("claude-pet: no valid pets found (built-ins failed to load). Exiting.")
            NSApp.terminate(nil)
            return
        }
        for problem in library.loadProblems {
            StandardError.print("claude-pet: skipping user pet — \(problem)")
        }

        viewModel = PetViewModel(
            pet: selectedPet.definition,
            pixelScale: CGFloat(config.pixelScale),
            isSpeechBubbleHidden: config.isSpeechBubbleHidden,
            isAlwaysFannedOut: config.isSessionsAlwaysExpanded
        )

        panel = OverlayPanel(contentSize: viewModel.contentSize, rootView: PetView(model: viewModel))
        panel.onClick = { [weak self] point in self?.viewModel.handleClick(atContentX: point.x) }
        panel.onRightClick = { [weak self] event, view in self?.showContextMenu(for: event, in: view) }
        panel.onDidMove = { [weak self] origin in
            guard let self else { return }
            // Store where the *collapsed* panel would sit, so a restart (always collapsed at first)
            // puts the primary pet back at the same spot even if we were fanned out when moved.
            let primaryScreenX = origin.x + self.viewModel.layout.primaryCenterX
            self.config.windowOriginX = primaryScreenX - self.viewModel.collapsedLayout.primaryCenterX
            self.config.windowOriginY = origin.y
            self.scheduleConfigSave()
        }
        panel.ignoresMouseEvents = config.isClickThrough

        var lastPrimaryCenterX = viewModel.layout.primaryCenterX
        viewModel.layoutDidChange
            .sink { [weak self] newLayout in
                guard let self else { return }
                // Keep the primary pet where it is on screen while the row grows or shrinks around it.
                let anchorScreenX = self.panel.frame.minX + lastPrimaryCenterX
                self.panel.resize(to: newLayout.contentSize, keepingContentX: newLayout.primaryCenterX, atScreenX: anchorScreenX)
                lastPrimaryCenterX = newLayout.primaryCenterX
            }
            .store(in: &cancellables)

        let savedOrigin: NSPoint? = {
            guard let x = config.windowOriginX, let y = config.windowOriginY else { return nil }
            return NSPoint(x: x, y: y)
        }()
        panel.place(at: savedOrigin)

        setUpStatusItem()
        viewModel.start()
        startSnapshotRequestWatcher()
        if !config.isPetHidden {
            panel.orderFrontRegardless()
        }
    }

    func applicationWillTerminate(_ notification: Notification) {
        viewModel?.stop()
        snapshotTimer?.invalidate()
        if configSaveTimer != nil {
            configSaveTimer?.invalidate()
            config.save()
        }
    }

    // MARK: - Debounced config save (didMove fires for every pixel while dragging)

    private var configSaveTimer: Timer?

    private func scheduleConfigSave() {
        configSaveTimer?.invalidate()
        let timer = Timer(timeInterval: 0.6, repeats: false) { [weak self] _ in
            Task { @MainActor [weak self] in
                guard let self else { return }
                self.configSaveTimer = nil
                self.config.save()
            }
        }
        RunLoop.main.add(timer, forMode: .common)
        configSaveTimer = timer
    }

    // MARK: - Snapshot requests (`claude-pet snapshot`)

    private var snapshotTimer: Timer?

    private func startSnapshotRequestWatcher() {
        let timer = Timer(timeInterval: 0.4, repeats: true) { [weak self] _ in
            Task { @MainActor [weak self] in self?.answerSnapshotRequestIfAny() }
        }
        RunLoop.main.add(timer, forMode: .common)
        snapshotTimer = timer
    }

    private func answerSnapshotRequestIfAny() {
        let clickURL = AppPaths.clickRequestFile
        if let data = try? Data(contentsOf: clickURL) {
            try? FileManager.default.removeItem(at: clickURL)
            let text = String(decoding: data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
            if let x = Double(text) {
                viewModel.handleClick(atContentX: CGFloat(x))
            } else if text == "primary" {
                viewModel.handleClick(atContentX: viewModel.layout.primaryCenterX)
            }
        }

        let requestURL = AppPaths.snapshotRequestFile
        guard FileManager.default.fileExists(atPath: requestURL.path) else { return }
        try? FileManager.default.removeItem(at: requestURL)
        do {
            try panel.writeSnapshot(to: AppPaths.snapshotImageFile)
        } catch {
            StandardError.print("claude-pet: snapshot failed: \(error.localizedDescription)")
        }
    }

    // MARK: - Status bar

    private func setUpStatusItem() {
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        if let button = statusItem.button {
            button.image = NSImage(systemSymbolName: "pawprint.fill", accessibilityDescription: "Claude Pet")
            button.image?.isTemplate = true
            button.toolTip = "Claude Pet"
        }
        let menu = NSMenu()
        menu.delegate = self
        statusItem.menu = menu
    }

    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        populateMenu(menu)
    }

    private func showContextMenu(for event: NSEvent, in view: NSView) {
        let menu = NSMenu()
        populateMenu(menu)
        NSMenu.popUpContextMenu(menu, with: event, for: view)
    }

    private func populateMenu(_ menu: NSMenu) {
        let headerTitle: String
        if let session = viewModel.focusedSession {
            headerTitle = "\(viewModel.pet.name) · \(session.projectName): \(viewModel.displayState.displayLabel)"
        } else {
            headerTitle = "\(viewModel.pet.name) · waiting for a Claude Code session"
        }
        let header = NSMenuItem(title: headerTitle, action: nil, keyEquivalent: "")
        header.isEnabled = false
        menu.addItem(header)

        if !viewModel.sessions.isEmpty {
            let sessionsMenu = NSMenu()
            let autoItem = makeItem("Automatic (approval > busy > recent)", action: #selector(followSessionsAutomatically))
            autoItem.state = viewModel.pinnedSessionId == nil ? .on : .off
            sessionsMenu.addItem(autoItem)
            sessionsMenu.addItem(.separator())
            for session in viewModel.sessions {
                let item = NSMenuItem(
                    title: "\(session.projectName) — \(session.effectiveState.displayLabel)",
                    action: #selector(pinSessionMenuItem(_:)),
                    keyEquivalent: ""
                )
                item.target = self
                item.representedObject = session.sessionId
                item.state = session.sessionId == viewModel.pinnedSessionId ? .on : .off
                item.toolTip = "\(session.cwd)\n\(session.message)\nClick to keep this session in front."
                sessionsMenu.addItem(item)
            }
            let sessionsItem = NSMenuItem(title: "Sessions (\(viewModel.sessions.count))", action: nil, keyEquivalent: "")
            sessionsItem.submenu = sessionsMenu
            menu.addItem(sessionsItem)
        }

        let expandItem = makeItem("Show all sessions side by side", action: #selector(toggleAlwaysExpanded))
        expandItem.state = config.isSessionsAlwaysExpanded ? .on : .off
        expandItem.toolTip = "Off: click the pet to fan sessions out temporarily. On: always show one pet per session."
        menu.addItem(expandItem)

        menu.addItem(.separator())

        // Pet picker
        let petMenu = NSMenu()
        for loadedPet in library.pets {
            let suffix = loadedPet.isBuiltIn ? "" : "  (custom)"
            let item = NSMenuItem(title: loadedPet.definition.name + suffix, action: #selector(selectPetMenuItem(_:)), keyEquivalent: "")
            item.target = self
            item.representedObject = loadedPet.definition.id
            item.state = loadedPet.definition.id == viewModel.pet.id ? .on : .off
            item.toolTip = loadedPet.definition.description
            petMenu.addItem(item)
        }
        petMenu.addItem(.separator())
        petMenu.addItem(makeItem("Reload pets", action: #selector(reloadPets)))
        petMenu.addItem(makeItem("Open pets folder…", action: #selector(openPetsFolder)))
        let petItem = NSMenuItem(title: "Pet", action: nil, keyEquivalent: "")
        petItem.submenu = petMenu
        menu.addItem(petItem)

        // Size picker (marks the option nearest to the current scale, so hand-edited configs still show a check)
        let sizeMenu = NSMenu()
        let nearestScale = AppConfig.availablePixelScales
            .min { abs(CGFloat($0.scale) - viewModel.pixelScale) < abs(CGFloat($1.scale) - viewModel.pixelScale) }?
            .scale
        for option in AppConfig.availablePixelScales {
            let item = NSMenuItem(title: option.label, action: #selector(selectSizeMenuItem(_:)), keyEquivalent: "")
            item.target = self
            item.representedObject = option.scale
            item.state = option.scale == nearestScale ? .on : .off
            sizeMenu.addItem(item)
        }
        let sizeItem = NSMenuItem(title: "Size", action: nil, keyEquivalent: "")
        sizeItem.submenu = sizeMenu
        menu.addItem(sizeItem)

        menu.addItem(.separator())

        let bubbleItem = makeItem("Hide speech bubbles", action: #selector(toggleSpeechBubbles))
        bubbleItem.state = config.isSpeechBubbleHidden ? .on : .off
        menu.addItem(bubbleItem)

        let clickThroughItem = makeItem("Click-through (ignore mouse)", action: #selector(toggleClickThrough))
        clickThroughItem.state = config.isClickThrough ? .on : .off
        clickThroughItem.toolTip = "When on, clicks pass through the pet. Use this menu bar item to turn it off again."
        menu.addItem(clickThroughItem)

        let hideItem = makeItem(config.isPetHidden ? "Show pet" : "Hide pet", action: #selector(togglePetHidden))
        menu.addItem(hideItem)

        menu.addItem(makeItem("Reset position", action: #selector(resetPosition)))

        menu.addItem(.separator())
        menu.addItem(makeItem("Install Claude Code hooks…", action: #selector(installHooks)))
        menu.addItem(makeItem("Open hook log", action: #selector(openHookLog)))
        menu.addItem(.separator())
        menu.addItem(makeItem("Quit Claude Pet", action: #selector(quit), keyEquivalent: "q"))
    }

    private func makeItem(_ title: String, action: Selector, keyEquivalent: String = "") -> NSMenuItem {
        let item = NSMenuItem(title: title, action: action, keyEquivalent: keyEquivalent)
        item.target = self
        return item
    }

    // MARK: - Menu actions

    @objc private func selectPetMenuItem(_ sender: NSMenuItem) {
        guard let petId = sender.representedObject as? String,
              let loadedPet = library.pet(withId: petId) else { return }
        viewModel.select(pet: loadedPet.definition)
        config.selectedPetId = petId
        config.save()
    }

    @objc private func selectSizeMenuItem(_ sender: NSMenuItem) {
        guard let scale = sender.representedObject as? Double else { return }
        viewModel.setPixelScale(CGFloat(scale))
        config.pixelScale = scale
        config.save()
    }

    @objc private func reloadPets() {
        library = PetLibrary.load()
        if let current = library.pet(withId: viewModel.pet.id) {
            viewModel.select(pet: current.definition)
        } else if let fallback = library.pets.first {
            viewModel.select(pet: fallback.definition)
        }
        for problem in library.loadProblems {
            StandardError.print("claude-pet: skipping user pet — \(problem)")
        }
    }

    @objc private func openPetsFolder() {
        try? AppPaths.ensureDirectoryExists(AppPaths.petsDirectory)
        NSWorkspace.shared.open(AppPaths.petsDirectory)
    }

    @objc private func pinSessionMenuItem(_ sender: NSMenuItem) {
        guard let sessionId = sender.representedObject as? String else { return }
        viewModel.pin(sessionId: sessionId)
    }

    @objc private func followSessionsAutomatically() {
        viewModel.pin(sessionId: nil)
    }

    @objc private func toggleAlwaysExpanded() {
        config.isSessionsAlwaysExpanded.toggle()
        viewModel.isAlwaysFannedOut = config.isSessionsAlwaysExpanded
        config.save()
    }

    @objc private func toggleSpeechBubbles() {
        config.isSpeechBubbleHidden.toggle()
        viewModel.isSpeechBubbleHidden = config.isSpeechBubbleHidden
        config.save()
    }

    @objc private func toggleClickThrough() {
        config.isClickThrough.toggle()
        panel.ignoresMouseEvents = config.isClickThrough
        config.save()
    }

    @objc private func togglePetHidden() {
        config.isPetHidden.toggle()
        if config.isPetHidden {
            panel.orderOut(nil)
        } else {
            panel.orderFrontRegardless()
        }
        config.save()
    }

    @objc private func resetPosition() {
        panel.setFrameOrigin(OverlayPanel.defaultOrigin(for: panel.frame.size))
    }

    @objc private func installHooks() {
        let alert = NSAlert()
        do {
            let report = try HooksInstaller().install()
            alert.messageText = "Claude Code hooks installed"
            alert.informativeText = report.summaryText
        } catch {
            alert.alertStyle = .warning
            alert.messageText = "Could not install hooks"
            alert.informativeText = error.localizedDescription
        }
        NSApp.activate(ignoringOtherApps: true)
        alert.runModal()
    }

    @objc private func openHookLog() {
        let logURL = AppPaths.hookLogFile
        if !FileManager.default.fileExists(atPath: logURL.path) {
            try? AppPaths.ensureDirectoryExists(logURL.deletingLastPathComponent())
            FileManager.default.createFile(atPath: logURL.path, contents: nil)
        }
        NSWorkspace.shared.open(logURL)
    }

    @objc private func quit() {
        NSApp.terminate(nil)
    }
}

/// Boots AppKit without an app bundle (`swift build` produces a bare executable).
enum OverlayApp {
    @MainActor
    static func run() -> Never {
        guard SingleInstanceLock.acquire() else {
            StandardError.print("claude-pet: the overlay is already running (lock: \(AppPaths.overlayLockFile.path)). Nothing to do.")
            exit(0)
        }
        let application = NSApplication.shared
        let delegate = AppDelegate()
        application.delegate = delegate
        application.run()
        exit(0)
    }
}

/// One overlay per user: a second `claude-pet run` (double-click, LaunchAgent + manual start)
/// exits immediately instead of stacking pets. Uses `flock`, so the lock vanishes with the process.
enum SingleInstanceLock {
    private static var lockFileDescriptor: Int32 = -1

    static func acquire() -> Bool {
        do {
            try AppPaths.ensureDirectoryExists(AppPaths.rootDirectory)
        } catch {
            return true // can't even create our dir; don't block startup over the lock
        }
        let descriptor = open(AppPaths.overlayLockFile.path, O_CREAT | O_RDWR | O_CLOEXEC, 0o644)
        guard descriptor >= 0 else { return true }
        if flock(descriptor, LOCK_EX | LOCK_NB) != 0 {
            close(descriptor)
            return false
        }
        lockFileDescriptor = descriptor // keep it open for the life of the process
        return true
    }
}
