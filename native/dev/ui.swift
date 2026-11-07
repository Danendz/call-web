// Drives the running app for UI checks on macOS without touching the real mouse or keyboard: elements are found by
// their accessible label (what `@tr` shows in the current language) and pressed through the Accessibility API, and
// screenshots capture the window even when it's behind others. Needs Accessibility and Screen Recording permission
// for the terminal running it.
//
//   swift dev/ui.swift tree                   accessible tree of the window: roles and labels
//   swift dev/ui.swift press <label>          press a button
//   swift dev/ui.swift set <label> <text>     set a text field's text
//   swift dev/ui.swift shot <file.png>        screenshot the window

import AppKit
import ApplicationServices

let appName = "call-danya"
let windowTitle = "Call Danya"

func fail(_ message: String) -> Never {
    FileHandle.standardError.write((message + "\n").data(using: .utf8)!)
    exit(1)
}

func attr(_ element: AXUIElement, _ name: String) -> AnyObject? {
    var value: AnyObject?
    return AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success ? value : nil
}

func labels(_ element: AXUIElement) -> [String] {
    [kAXTitleAttribute, kAXDescriptionAttribute, kAXValueAttribute]
        .compactMap { attr(element, $0) as? String }
        .filter { !$0.isEmpty }
}

/// Depth-first; stops when `visit` returns true. Skips the menu bar, which is the system's, not ours. The depth cap
/// keeps a cycle in the tree from recursing forever.
@discardableResult
func walk(_ element: AXUIElement, depth: Int = 0, _ visit: (AXUIElement, Int) -> Bool) -> Bool {
    if depth > 40 { return false }
    if visit(element, depth) { return true }
    for child in (attr(element, kAXChildrenAttribute) as? [AXUIElement]) ?? [] {
        if (attr(child, kAXRoleAttribute) as? String) == kAXMenuBarRole { continue }
        if walk(child, depth: depth + 1, visit) { return true }
    }
    return false
}

func appElement() -> AXUIElement {
    guard AXIsProcessTrusted() else { fail("give this terminal Accessibility permission in System Settings") }
    guard let app = NSWorkspace.shared.runningApplications.first(where: { $0.localizedName == appName }) else {
        fail("\(appName) isn't running")
    }
    let root = AXUIElementCreateApplication(app.processIdentifier)
    // AccessKit builds the tree on the first request, so the first walk can come back empty
    for _ in 0..<10 {
        var hasButton = false
        walk(root) { element, _ in
            hasButton = (attr(element, kAXRoleAttribute) as? String) == kAXButtonRole && !labels(element).isEmpty
            return hasButton
        }
        if hasButton { break }
        usleep(200_000)
    }
    return root
}

func find(_ label: String) -> AXUIElement {
    var found: AXUIElement?
    walk(appElement()) { element, _ in
        if labels(element).contains(label) { found = element }
        return found != nil
    }
    guard let element = found else { fail("nothing labelled \"\(label)\"; see `tree`") }
    return element
}

func screenshot(_ path: String) {
    let windows = CGWindowListCopyWindowInfo(.optionAll, kCGNullWindowID) as? [[String: Any]] ?? []
    guard let id = windows.first(where: {
        $0[kCGWindowOwnerName as String] as? String == appName && $0[kCGWindowName as String] as? String == windowTitle
    })?[kCGWindowNumber as String] as? Int else { fail("no \(windowTitle) window (Screen Recording permission?)") }
    let capture = Process()
    capture.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
    capture.arguments = ["-x", "-o", "-l\(id)", path]
    do { try capture.run() } catch { fail("screencapture: \(error)") }
    capture.waitUntilExit()
    if capture.terminationStatus != 0 { fail("screencapture failed") }
}

let args = Array(CommandLine.arguments.dropFirst())
switch (args.first, args.count) {
case ("tree", 1):
    walk(appElement()) { element, depth in
        let role = (attr(element, kAXRoleAttribute) as? String) ?? "?"
        print(String(repeating: "  ", count: depth) + ([role] + labels(element)).joined(separator: "  "))
        return false
    }
case ("press", 2):
    let result = AXUIElementPerformAction(find(args[1]), kAXPressAction as CFString)
    if result != .success { fail("press failed (\(result.rawValue))") }
case ("set", 3):
    let result = AXUIElementSetAttributeValue(find(args[1]), kAXValueAttribute as CFString, args[2] as CFString)
    if result != .success { fail("set failed (\(result.rawValue))") }
case ("shot", 2):
    screenshot(args[1])
default:
    fail("usage: swift dev/ui.swift tree | press <label> | set <label> <text> | shot <file.png>")
}
