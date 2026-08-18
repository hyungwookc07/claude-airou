import AppKit
import SwiftUI

/// A borderless, transparent, always-on-top panel that never steals focus.
/// Dragging anywhere on it moves it; a plain click "pets" the creature; right-click opens the menu.
final class OverlayPanel: NSPanel {
    /// Called with the click location in content coordinates (points from the left edge).
    var onClick: ((NSPoint) -> Void)?
    var onRightClick: ((NSEvent, NSView) -> Void)?
    var onDidMove: ((NSPoint) -> Void)?

    private let containerView: PetContainerView

    init<Content: View>(contentSize: CGSize, rootView: Content) {
        containerView = PetContainerView(frame: NSRect(origin: .zero, size: contentSize))
        super.init(
            contentRect: NSRect(origin: .zero, size: contentSize),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )

        isFloatingPanel = true
        level = .floating
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary, .ignoresCycle]
        isOpaque = false
        backgroundColor = .clear
        hasShadow = false
        hidesOnDeactivate = false
        isReleasedWhenClosed = false
        isMovableByWindowBackground = false // we drive dragging ourselves so clicks stay distinguishable
        becomesKeyOnlyIfNeeded = true
        animationBehavior = .none
        titleVisibility = .hidden
        titlebarAppearsTransparent = true

        let hostingView = NSHostingView(rootView: rootView)
        hostingView.translatesAutoresizingMaskIntoConstraints = false
        containerView.addSubview(hostingView)
        NSLayoutConstraint.activate([
            hostingView.leadingAnchor.constraint(equalTo: containerView.leadingAnchor),
            hostingView.trailingAnchor.constraint(equalTo: containerView.trailingAnchor),
            hostingView.topAnchor.constraint(equalTo: containerView.topAnchor),
            hostingView.bottomAnchor.constraint(equalTo: containerView.bottomAnchor),
        ])
        contentView = containerView

        containerView.onClick = { [weak self] point in self?.onClick?(point) }
        containerView.onRightClick = { [weak self] event, view in self?.onRightClick?(event, view) }
        containerView.onDragStarted = { [weak self] event in self?.performDrag(with: event) }

        NotificationCenter.default.addObserver(
            self,
            selector: #selector(handleDidMove(_:)),
            name: NSWindow.didMoveNotification,
            object: self
        )
    }

    deinit {
        NotificationCenter.default.removeObserver(self)
    }

    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }

    @objc private func handleDidMove(_ notification: Notification) {
        onDidMove?(frame.origin)
    }

    // MARK: - Placement

    /// Moves the panel to `origin` if that keeps it on some screen; otherwise to the default corner.
    func place(at origin: NSPoint?) {
        if let origin, Self.isRectVisible(NSRect(origin: origin, size: frame.size)) {
            setFrameOrigin(origin)
        } else {
            setFrameOrigin(Self.defaultOrigin(for: frame.size))
        }
    }

    /// Bottom-right corner of the main screen's visible area, with a margin.
    static func defaultOrigin(for size: CGSize) -> NSPoint {
        let visible = (NSScreen.main ?? NSScreen.screens.first)?.visibleFrame ?? NSRect(x: 0, y: 0, width: 1280, height: 800)
        let margin: CGFloat = 24
        return NSPoint(x: visible.maxX - size.width - margin, y: visible.minY + margin)
    }

    static func isRectVisible(_ rect: NSRect) -> Bool {
        // Require a meaningful chunk of the panel to intersect a screen so it stays reachable.
        let probe = rect.insetBy(dx: rect.width * 0.25, dy: rect.height * 0.25)
        return NSScreen.screens.contains { $0.visibleFrame.intersects(probe) }
    }

    /// Renders the panel's own content to a PNG (no screen-recording permission needed).
    func writeSnapshot(to url: URL) throws {
        let bounds = containerView.bounds
        guard let bitmap = containerView.bitmapImageRepForCachingDisplay(in: bounds) else {
            throw SpriteRenderer.RenderError(message: "could not create bitmap for snapshot")
        }
        containerView.cacheDisplay(in: bounds, to: bitmap)
        guard let data = bitmap.representation(using: .png, properties: [:]) else {
            throw SpriteRenderer.RenderError(message: "could not encode snapshot PNG")
        }
        try data.write(to: url, options: .atomic)
    }

    func resizeKeepingBottomLeft(to size: CGSize) {
        var newFrame = frame
        newFrame.size = size
        setFrame(newFrame, display: false, animate: false)
        nudgeOntoScreen()
    }

    /// Resizes so that the content point `contentX` (points from the left) stays at the same
    /// screen x — used to keep the primary pet still while the row fans out around it.
    func resize(to size: CGSize, keepingContentX contentX: CGFloat, atScreenX screenX: CGFloat) {
        let newFrame = NSRect(x: (screenX - contentX).rounded(), y: frame.minY, width: size.width, height: size.height)
        // display: false — let the next display pass draw the *new* SwiftUI content into the new frame
        // instead of flashing the old content centred in it for one frame.
        setFrame(newFrame, display: false, animate: false)
        nudgeOntoScreen()
    }

    /// Slides the panel back inside the nearest screen's visible area if it grew or was
    /// restored partly off-screen; falls back to the default corner if no screen is near.
    func nudgeOntoScreen() {
        let current = frame
        let screen = NSScreen.screens.min { lhs, rhs in
            Self.distance(from: current, to: lhs.visibleFrame) < Self.distance(from: current, to: rhs.visibleFrame)
        }
        guard let visible = screen?.visibleFrame else { return }
        var origin = current.origin
        origin.x = min(max(origin.x, visible.minX), max(visible.minX, visible.maxX - current.width))
        origin.y = min(max(origin.y, visible.minY), max(visible.minY, visible.maxY - current.height))
        if origin != current.origin {
            setFrameOrigin(origin)
        }
    }

    private static func distance(from rect: NSRect, to other: NSRect) -> CGFloat {
        let dx = max(other.minX - rect.maxX, rect.minX - other.maxX, 0)
        let dy = max(other.minY - rect.maxY, rect.minY - other.maxY, 0)
        return dx * dx + dy * dy
    }
}

/// Container that swallows all mouse events (so SwiftUI never fights us over them) and
/// turns them into click / drag / right-click callbacks.
final class PetContainerView: NSView {
    var onClick: ((NSPoint) -> Void)?
    var onRightClick: ((NSEvent, NSView) -> Void)?
    var onDragStarted: ((NSEvent) -> Void)?

    private var mouseDownEvent: NSEvent?
    private var didDrag = false
    private let dragThreshold: CGFloat = 3

    override func hitTest(_ point: NSPoint) -> NSView? {
        // Everything inside the panel is "the pet"; keep events here.
        bounds.contains(point) ? self : nil
    }

    override var acceptsFirstResponder: Bool { false }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func mouseDown(with event: NSEvent) {
        if event.modifierFlags.contains(.control) {
            // Control-click is the keyboard-only way to get a context menu; treat it like right-click.
            mouseDownEvent = nil
            onRightClick?(event, self)
            return
        }
        mouseDownEvent = event
        didDrag = false
    }

    override func mouseDragged(with event: NSEvent) {
        guard !didDrag, let downEvent = mouseDownEvent else { return }
        let start = downEvent.locationInWindow
        let current = event.locationInWindow
        if abs(current.x - start.x) > dragThreshold || abs(current.y - start.y) > dragThreshold {
            didDrag = true
            // AppKit asks for the original mouse-down event here (see performDrag(with:) docs).
            onDragStarted?(downEvent)
        }
    }

    override func mouseUp(with event: NSEvent) {
        if !didDrag, mouseDownEvent != nil {
            // Content coordinates: x from the left, y from the top (matches SwiftUI's layout space).
            let location = convert(event.locationInWindow, from: nil)
            OverlayLog.append("mouseUp window=(\(Int(event.locationInWindow.x)),\(Int(event.locationInWindow.y))) view=(\(Int(location.x)),\(Int(location.y))) bounds=\(Int(bounds.width))x\(Int(bounds.height)) frame=\(Int(frame.origin.x)),\(Int(frame.origin.y)) windowFrame=\(window.map { "\(Int($0.frame.minX)),\(Int($0.frame.minY)) \(Int($0.frame.width))x\(Int($0.frame.height))" } ?? "-")")
            onClick?(NSPoint(x: location.x, y: bounds.height - location.y))
        }
        mouseDownEvent = nil
        didDrag = false
    }

    override func rightMouseDown(with event: NSEvent) {
        onRightClick?(event, self)
    }
}
