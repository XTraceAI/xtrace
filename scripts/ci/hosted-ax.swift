import AppKit
import ApplicationServices

// External, read/press-only Accessibility driver. No injected JS, product IPC,
// TCC changes, permission prompts or app launch/termination commands.
func finish(_ code: Int32, _ reason: String, _ nodes: [[String: String]] = []) -> Never {
    let report: [String: Any] = ["code": code, "reason": reason, "nodes": nodes]
    if let data = try? JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys]) {
        FileHandle.standardOutput.write(data)
        print("")
    }
    exit(code)
}

guard CommandLine.arguments.count == 5,
      let pid = Int32(CommandLine.arguments[1]), pid > 0 else { finish(2, "Invalid arguments") }
let bundle = URL(fileURLWithPath: CommandLine.arguments[2]).standardizedFileURL.resolvingSymlinksInPath()
let identifier = CommandLine.arguments[3]
let mode = CommandLine.arguments[4]
let releaseMode = mode == "first-launch"
let authorization = ProcessInfo.processInfo.environment
let releaseAuthorized = releaseMode && identifier == "ai.xtrace.app" &&
    bundle.path == "/Applications/XTrace Desktop.app" &&
    authorization["ALLOW_RELEASE_IDENTITY"] == "true" &&
    authorization["GITHUB_ACTIONS"] == "true" &&
    authorization["GITHUB_EVENT_NAME"] == "workflow_dispatch" &&
    authorization["RUNNER_ENVIRONMENT"] == "github-hosted" &&
    ProcessInfo.processInfo.operatingSystemVersion.majorVersion == 14
let probeAuthorized = identifier.range(of: #"^ai\.xtrace\.app\.test\.[a-f0-9-]+$"#,
                       options: .regularExpression) != nil &&
    bundle.lastPathComponent == "Hosted AX Probe.app" && (mode == "missing-control" || mode == "press")
guard releaseAuthorized || probeAuthorized else { finish(2, "Unsupported identity or mode") }
#if !arch(arm64)
if releaseMode { finish(2, "Release mode requires arm64") }
#endif
guard AXIsProcessTrusted() else { finish(4, "Accessibility is unavailable; no permission changes attempted") }

func ownedApplication() -> NSRunningApplication? {
    guard let app = NSRunningApplication(processIdentifier: pid), !app.isTerminated,
          app.bundleIdentifier == identifier,
          app.bundleURL?.standardizedFileURL.resolvingSymlinksInPath() == bundle,
          app.executableURL?.standardizedFileURL.resolvingSymlinksInPath() ==
              bundle.appendingPathComponent(releaseMode ? "Contents/MacOS/xtrace-desktop" : "Contents/MacOS/HostedAXProbe") else { return nil }
    return app
}

func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success else { return nil }
    return value
}

func snapshot(_ root: AXUIElement) -> [(AXUIElement, [String: String])] {
    var result: [(AXUIElement, [String: String])] = []
    var queue: [(AXUIElement, Int)] = [(root, 0)]
    var offset = 0
    while offset < queue.count && result.count < 1000 {
        let (element, depth) = queue[offset]
        offset += 1
        var row: [String: String] = [:]
        for name in [kAXRoleAttribute, kAXSubroleAttribute, kAXTitleAttribute, kAXValueAttribute, kAXDescriptionAttribute] {
            if let value = attribute(element, name) as? String { row[name] = String(value.prefix(300)) }
        }
        result.append((element, row))
        if depth < 20, let children = attribute(element, kAXChildrenAttribute) as? [AXUIElement] {
            queue.append(contentsOf: children.prefix(1000 - queue.count).map { ($0, depth + 1) })
        }
    }
    return result
}

let root = AXUIElementCreateApplication(pid)
AXUIElementSetMessagingTimeout(root, 0.5)
let deadline = Date().addingTimeInterval(releaseMode ? 90 : 25)
let title = releaseMode ? "Open Dashboard" : mode == "press" ? "Confirm AX action" : "This control must not exist"
var pressed = false
var sawWindow = false
var sawIdentity = false
var negativeSince: Date?
var last: [[String: String]] = []
var welcomeEvidence: [[String: String]] = []
while Date() < deadline {
    guard let app = ownedApplication() else {
        if sawIdentity { finish(5, "Owned process identity was lost", last) }
        RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        continue
    }
    sawIdentity = true
    if app.isFinishedLaunching {
        let elements = snapshot(root)
        last = elements.map { $0.1 }
        sawWindow = last.contains { $0[kAXRoleAttribute] == kAXWindowRole }
        let sawWebContent = last.contains { $0[kAXRoleAttribute] == "AXWebArea" }
        let hasText: (String) -> Bool = { text in last.contains { $0.values.contains(text) } }
        let button = elements.first(where: {
            $0.1[kAXRoleAttribute] == kAXButtonRole &&
                [$0.1[kAXTitleAttribute], $0.1[kAXDescriptionAttribute]].contains(title)
        })?.0
        if releaseMode {
            let errors = ["Local index status and database counts could not be read.",
                "Local index status could not be read.", "Database counts could not be read.",
                "Dashboard metrics could not be loaded.", "The local index has stopped."]
            if last.contains(where: { row in
                row[kAXRoleAttribute] == "AXAlert" || row[kAXSubroleAttribute] == "AXDialog" ||
                    row.values.contains(where: { text in errors.contains(where: { text.contains($0) }) })
            }) { finish(6, "The released app displayed a startup or native-data error", last) }
            if sawWindow, sawWebContent, !pressed, let button,
               hasText("Welcome to XTrace"), hasText("The initial scan finished."),
               hasText("Bundled readers verified"), hasText("v0.1.6"),
               hasText("In the local database now"),
               !last.contains(where: { $0.values.contains(where: { $0.contains("Reading database counts") }) }) {
                guard ownedApplication() != nil else { finish(5, "Identity changed before press", last) }
                welcomeEvidence = last
                guard AXUIElementPerformAction(button, kAXPressAction as CFString) == .success else {
                    finish(6, "Open Dashboard AXPress failed", last)
                }
                pressed = true
            } else if sawWindow, sawWebContent, pressed, hasText("v0.1.6"),
                      !hasText("Welcome to XTrace"), hasText("What your agents did"),
                      hasText("No agent activity was recorded in this range. Measured zeros are shown as 0.") {
                finish(0, "Observed native-ready Welcome, Open Dashboard action and usable 0.1.6 Dashboard",
                       [["evidence": "Welcome before action"]] + welcomeEvidence +
                       [["evidence": "Dashboard after action"]] + last)
            }
        } else if mode == "missing-control" {
            guard button == nil else { finish(6, "Negative control unexpectedly exists", last) }
            if sawWindow, sawWebContent, hasText("Awaiting AX action"), !hasText("AX action confirmed") {
                if negativeSince == nil { negativeSince = Date() }
                if Date().timeIntervalSince(negativeSince!) >= 3 {
                    finish(3, "Missing control timed out without changing the fixture", last)
                }
            } else { negativeSince = nil }
        } else if sawWindow, sawWebContent, !pressed, let button {
            guard mode == "press" else { finish(6, "Negative control unexpectedly exists", last) }
            guard hasText("Awaiting AX action"), !hasText("AX action confirmed") else {
                finish(6, "Fixture did not start in its untouched state", last)
            }
            guard ownedApplication() != nil else { finish(5, "Identity changed before press", last) }
            guard AXUIElementPerformAction(button, kAXPressAction as CFString) == .success else {
                finish(6, "AXPress failed", last)
            }
            pressed = true
        } else if sawWindow, sawWebContent, pressed, hasText("AX action confirmed"), !hasText("Awaiting AX action") {
            finish(0, "Observed window, AXPress and changed visible label", last)
        }
    }
    RunLoop.current.run(until: Date().addingTimeInterval(0.1))
}
finish(6, "Timed out without the required rendered UI result", last)
