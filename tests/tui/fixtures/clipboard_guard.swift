import AppKit
import Foundation

// macOS-only wrapper for `npm run test:clipboard`. Texo writes to the real system
// clipboard, so the test would otherwise replace whatever the user had copied.
// Swift gives us AppKit's pasteboard API to save and restore images, rich text,
// and other available formats; a plain-text clipboard backup would lose them.
// Keep the original contents in memory and restore them when the child exits,
// including when a test fails. Never write the user's clipboard contents to disk.
func runGuardedTests() throws -> Int32 {
    let pasteboard = NSPasteboard.general
    let savedItems = (pasteboard.pasteboardItems ?? []).map { item in
        let saved = NSPasteboardItem()
        for type in item.types {
            if let data = item.data(forType: type) {
                saved.setData(data, forType: type)
            }
        }
        return saved
    }

    defer {
        pasteboard.clearContents()
        if !savedItems.isEmpty {
            pasteboard.writeObjects(savedItems)
        }
    }

    let child = Process()
    child.executableURL = URL(fileURLWithPath: "/usr/bin/env")
    child.arguments = Array(CommandLine.arguments.dropFirst())
    var environment = ProcessInfo.processInfo.environment
    environment["TEXO_CLIPBOARD_GUARDED"] = "1"
    child.environment = environment
    try child.run()
    child.waitUntilExit()
    return child.terminationStatus
}

do {
    exit(try runGuardedTests())
} catch {
    fputs("Could not run the clipboard test: \(error)\n", stderr)
    exit(1)
}
