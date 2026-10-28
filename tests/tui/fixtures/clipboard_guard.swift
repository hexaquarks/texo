import AppKit
import Foundation

// Preserve every pasteboard format in memory, including images and rich text.
// The child tests use the real OS clipboard; never write its original contents to disk.
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
