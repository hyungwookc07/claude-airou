import Foundation

// `claude-pet hook` must be as cheap as possible: it runs on every Claude Code event.
// Everything that is not the overlay app returns an exit code from `dispatch`; the overlay
// takes over the process (`OverlayApp.run()` never returns).
let arguments = Array(CommandLine.arguments.dropFirst())
if let exitCode = CommandLineInterface.dispatch(arguments: arguments) {
    exit(exitCode)
}
// Top-level code runs on the main thread; tell the compiler so.
MainActor.assumeIsolated {
    OverlayApp.run()
}
