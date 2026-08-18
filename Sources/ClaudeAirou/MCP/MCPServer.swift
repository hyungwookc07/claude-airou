import Foundation

/// Entry point for `claude-airou mcp`: a stdio MCP (Model Context Protocol) server so
/// Claude chat — the Claude desktop app — can drive the pet. Chat has no hook system,
/// so instead of observing events the pet exposes tools (`pet_status`, `hatch_pet`, …)
/// that Claude calls while it works. State is written to the same
/// `~/.claude-airou/state/<session>.json` files the hook uses; the overlay needs no changes.
///
/// Transport: newline-delimited JSON-RPC 2.0 on stdin/stdout (the desktop app's stdio
/// transport). stdout carries protocol messages only; diagnostics go to ~/.claude-airou/mcp.log.
enum MCPServerCommand {
    static func run() -> Int32 {
        MCPServer().serve()
    }
}

final class MCPServer {
    /// Versions we can speak. An unknown client version is answered with the latest.
    static let supportedProtocolVersions: Set<String> = ["2024-11-05", "2025-03-26", "2025-06-18"]
    static let latestProtocolVersion = "2025-06-18"
    /// Chat has no Stop event, so a busy state left behind (Claude never called `pet_status`
    /// again) is reset to idle by the watchdog after this long. Attention states keep the
    /// longer decay from `PetState.transientDurationSeconds`, same as Claude Code sessions.
    static let busyIdleAfterSeconds: TimeInterval = 3 * 60
    static let logMaxBytes: UInt64 = 512 * 1024

    private let stateStore: SessionStateStore
    private let sessionId: String
    private let stateLock = NSLock()
    /// Shown as the session's "project name" in the overlay. Refined from clientInfo on initialize.
    private var sessionLabel = "Claude Chat"
    private var lastWrittenState: PetState?
    private var lastWriteAt = Date()
    private var didCleanUp = false
    private let backgroundQueue = DispatchQueue(label: "claude-airou.mcp.background")
    private var watchdogTimer: DispatchSourceTimer?
    private var signalSources: [DispatchSourceProtocol] = []

    init(stateStore: SessionStateStore = SessionStateStore()) {
        self.stateStore = stateStore
        // One server process = one chat session (the app launches it once and keeps it running).
        self.sessionId = "claude-chat-\(ProcessInfo.processInfo.processIdentifier)"
    }

    // MARK: - Main loop

    func serve() -> Int32 {
        installSignalHandlers()
        startIdleWatchdog()
        log("started (session \(sessionId), pid \(ProcessInfo.processInfo.processIdentifier))")

        var buffer = Data()
        let standardInput = FileHandle.standardInput
        while true {
            let chunk = standardInput.availableData
            if chunk.isEmpty { break } // EOF: the client is gone
            buffer.append(chunk)
            while let newlineIndex = buffer.firstIndex(of: UInt8(ascii: "\n")) {
                let lineData = buffer.subdata(in: buffer.startIndex..<newlineIndex)
                buffer.removeSubrange(buffer.startIndex...newlineIndex)
                handleLine(lineData)
            }
        }
        cleanUpSession(reason: "stdin closed")
        return 0
    }

    private func handleLine(_ lineData: Data) {
        guard let line = String(data: lineData, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines),
              !line.isEmpty else { return }
        guard let parsed = try? JSONSerialization.jsonObject(with: Data(line.utf8)) else {
            log("unparseable message (\(lineData.count) bytes)")
            send(errorResponse(id: NSNull(), code: -32700, message: "Parse error"))
            return
        }
        if let object = parsed as? [String: Any] {
            handleMessage(object)
        } else if let batch = parsed as? [[String: Any]] {
            // Batching was removed from the protocol in 2025-06-18 but old clients may send it.
            for object in batch { handleMessage(object) }
        } else {
            send(errorResponse(id: NSNull(), code: -32600, message: "Invalid request"))
        }
    }

    private func handleMessage(_ message: [String: Any]) {
        // A response to a server-initiated request; we never send any, so nothing to match.
        guard let method = message["method"] as? String else { return }
        let id = message["id"]
        let params = message["params"] as? [String: Any] ?? [:]

        switch method {
        case "initialize":
            handleInitialize(id: id, params: params)
        case "ping":
            if let id { send(response(id: id, result: [:])) }
        case "tools/list":
            if let id { send(response(id: id, result: ["tools": MCPPetTools.descriptors])) }
        case "tools/call":
            handleToolCall(id: id, params: params)
        case let name where name.hasPrefix("notifications/"):
            // initialized / cancelled / roots changed — nothing to do, but never an error.
            log("notification \(name)")
        default:
            log("unknown method \(method)")
            if let id { send(errorResponse(id: id, code: -32601, message: "Method not found: \(method)")) }
        }
    }

    // MARK: - Requests

    private func handleInitialize(id: Any?, params: [String: Any]) {
        let clientInfo = params["clientInfo"] as? [String: Any] ?? [:]
        let clientName = clientInfo["name"] as? String ?? ""
        let label = Self.sessionLabel(forClientName: clientName)
        stateLock.lock()
        sessionLabel = label
        stateLock.unlock()
        log("initialize from \"\(clientName)\" → label \"\(label)\"")

        let requestedVersion = params["protocolVersion"] as? String ?? ""
        let version = Self.supportedProtocolVersions.contains(requestedVersion)
            ? requestedVersion
            : Self.latestProtocolVersion

        writeState(.hello, message: "Hi! Ready when you are", event: "mcp:initialize")

        guard let id else { return }
        send(response(id: id, result: [
            "protocolVersion": version,
            "capabilities": ["tools": [:] as [String: Any]],
            "serverInfo": ["name": "claude-airou", "version": CommandLineInterface.version],
            "instructions": """
            This server controls the user's claude-airou desktop pet — a small pixel companion \
            floating on their screen that mirrors what Claude is doing. Keep it honest: call \
            pet_status("thinking" or "working") when you start on a request or a long step, \
            pet_status("done") when you finish, pet_status("error") when something fails, and \
            pet_status("needs_input") when you are waiting for the user's answer. Speech-bubble \
            messages should stay under 60 characters. Use hatch_pet to create or edit custom \
            pets when the user asks for one.
            """,
        ]))
    }

    private func handleToolCall(id: Any?, params: [String: Any]) {
        let toolName = params["name"] as? String ?? ""
        let arguments = params["arguments"] as? [String: Any] ?? [:]
        guard let result = MCPPetTools.call(name: toolName, arguments: arguments, server: self) else {
            if let id { send(errorResponse(id: id, code: -32602, message: "Unknown tool: \(toolName)")) }
            return
        }
        log("tools/call \(toolName)\(result.isError ? " (error)" : "")")
        // After any non-status tool Claude reads the result and keeps composing its reply.
        if toolName != MCPPetTools.petStatusToolName, !result.isError {
            writeState(.thinking, message: "Thinking…", event: "mcp:tools/call:\(toolName)")
        }
        guard let id else { return }
        send(response(id: id, result: [
            "content": result.content,
            "isError": result.isError,
        ]))
    }

    /// "claude-ai" (the desktop app) → "Claude Chat"; other MCP clients keep their own name.
    static func sessionLabel(forClientName clientName: String) -> String {
        let lowered = clientName.lowercased()
        if lowered.isEmpty { return "Claude Chat" }
        if lowered.contains("claude") {
            return lowered.contains("code") ? "Claude Code" : "Claude Chat"
        }
        return clientName
    }

    // MARK: - Session state

    /// Writes the session snapshot the overlay reads. Same file format as the hook;
    /// `cwd` carries the display label (the overlay shows its last path component).
    func writeState(_ state: PetState, message: String, event: String) {
        stateLock.lock()
        defer { stateLock.unlock() }
        let snapshot = SessionSnapshot(
            sessionId: sessionId,
            cwd: sessionLabel,
            state: state,
            message: message,
            lastEventName: event,
            toolName: nil,
            updatedAtEpochSeconds: Date().timeIntervalSince1970
        )
        do {
            try stateStore.write(snapshot)
            lastWrittenState = state
            lastWriteAt = Date()
            log("\(event) -> \(state.rawValue) \"\(message)\"")
        } catch {
            log("\(event) write failed: \(error.localizedDescription)")
        }
    }

    private func cleanUpSession(reason: String) {
        stateLock.lock()
        let alreadyDone = didCleanUp
        didCleanUp = true
        stateLock.unlock()
        guard !alreadyDone else { return }
        stateStore.remove(sessionId: sessionId)
        log("stopped (\(reason))")
    }

    // MARK: - Watchdog & signals

    private func startIdleWatchdog() {
        let timer = DispatchSource.makeTimerSource(queue: backgroundQueue)
        timer.schedule(deadline: .now() + 30, repeating: 30)
        timer.setEventHandler { [weak self] in self?.idleWatchdogTick() }
        timer.resume()
        watchdogTimer = timer
    }

    private func idleWatchdogTick() {
        stateLock.lock()
        let state = lastWrittenState
        let age = Date().timeIntervalSince(lastWriteAt)
        stateLock.unlock()
        guard let state, state.isBusy, age > Self.busyIdleAfterSeconds else { return }
        writeState(.idle, message: "", event: "mcp:idle-watchdog")
    }

    /// The desktop app terminates servers when it quits; remove the session so the
    /// overlay does not keep a ghost "Claude Chat" pet around.
    private func installSignalHandlers() {
        for signalNumber in [SIGTERM, SIGINT, SIGHUP] {
            signal(signalNumber, SIG_IGN)
            let source = DispatchSource.makeSignalSource(signal: signalNumber, queue: backgroundQueue)
            source.setEventHandler { [weak self] in
                self?.cleanUpSession(reason: "signal \(signalNumber)")
                exit(0)
            }
            source.resume()
            signalSources.append(source)
        }
    }

    // MARK: - JSON-RPC plumbing

    private func response(id: Any, result: [String: Any]) -> [String: Any] {
        ["jsonrpc": "2.0", "id": id, "result": result]
    }

    private func errorResponse(id: Any, code: Int, message: String) -> [String: Any] {
        let error: [String: Any] = ["code": code, "message": message]
        return ["jsonrpc": "2.0", "id": id, "error": error]
    }

    /// One JSON object per line. `JSONSerialization` without `.prettyPrinted` never emits
    /// newlines, which is exactly what the stdio transport needs.
    private func send(_ object: [String: Any]) {
        guard JSONSerialization.isValidJSONObject(object),
              let data = try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys, .withoutEscapingSlashes]) else {
            log("could not serialize response")
            return
        }
        FileHandle.standardOutput.write(data + Data("\n".utf8))
    }

    // MARK: - Logging

    /// Appends one line to `~/.claude-airou/mcp.log` (stdout belongs to the protocol).
    func log(_ line: String) {
        Self.appendLog(line)
    }

    private static let logTimestampFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyy-MM-dd HH:mm:ss"
        return formatter
    }()

    static func appendLog(_ line: String) {
        let logURL = AppPaths.mcpLogFile
        do {
            try AppPaths.ensureDirectoryExists(logURL.deletingLastPathComponent())
            let fileManager = FileManager.default
            if let attributes = try? fileManager.attributesOfItem(atPath: logURL.path),
               let size = attributes[.size] as? UInt64, size > logMaxBytes {
                try? fileManager.removeItem(at: logURL)
            }
            if !fileManager.fileExists(atPath: logURL.path) {
                fileManager.createFile(atPath: logURL.path, contents: nil)
            }
            let handle = try FileHandle(forWritingTo: logURL)
            defer { try? handle.close() }
            try handle.seekToEnd()
            try handle.write(contentsOf: Data("\(logTimestampFormatter.string(from: Date())) \(line)\n".utf8))
        } catch {
            // Logging is best-effort.
        }
    }
}
