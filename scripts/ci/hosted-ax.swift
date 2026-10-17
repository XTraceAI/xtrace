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
let updateMode = ["update-ready", "update-press", "updated-dashboard"].contains(mode)
let releaseMode = mode == "first-launch" || updateMode
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
    // Parent IDs and sibling positions describe this snapshot only. Keep original
    // child positions even when the existing traversal limits truncate a branch.
    var queue: [(AXUIElement, Int, String, Int, Bool)] = [(root, 0, "", 0, false)]
    var offset = 0
    while offset < queue.count && result.count < 1000 {
        let nodeID = String(offset)
        let (element, depth, parentID, childIndex, inheritedWorkspace) = queue[offset]
        offset += 1
        var row: [String: String] = [:]
        for name in [kAXRoleAttribute, kAXSubroleAttribute, kAXTitleAttribute, kAXValueAttribute, kAXDescriptionAttribute, kAXHelpAttribute] {
            if let value = attribute(element, name) as? String { row[name] = String(value.prefix(300)) }
        }
        let inWorkspace = inheritedWorkspace ||
            (row[kAXRoleAttribute] == "AXGroup" && row[kAXSubroleAttribute] == "AXLandmarkComplementary" &&
             row[kAXDescriptionAttribute] == "Workspace")
        row["captureNode"] = nodeID
        row["captureParent"] = parentID
        row["captureChildIndex"] = String(childIndex)
        row["captureWorkspace"] = inWorkspace ? "true" : "false"
        result.append((element, row))
        if depth < 20, let children = attribute(element, kAXChildrenAttribute) as? [AXUIElement] {
            queue.append(contentsOf: children.enumerated().prefix(1000 - queue.count).map {
                ($0.element, depth + 1, nodeID, $0.offset, inWorkspace)
            })
        }
    }
    return result
}

// BEGIN UPDATE UI POLICY: offline controls call these with synthetic rows.
func hasText(_ rows: [[String: String]], _ text: String) -> Bool {
    rows.contains { $0.values.contains(text) }
}
// WebKit may expose v{version} as one text item or two adjacent siblings.
// Only the Workspace sidebar's displayed static text can establish app version.
func displayedVersion(_ rows: [[String: String]], _ version: String) -> Bool {
    for row in rows where row[kAXRoleAttribute] == "AXStaticText" && row["captureWorkspace"] == "true" {
        if row[kAXValueAttribute] == "v" + version { return true }
        guard row[kAXValueAttribute] == "v", let parent = row["captureParent"], !parent.isEmpty,
              let position = Int(row["captureChildIndex"] ?? ""), position >= 0, position < 1000 else { continue }
        if rows.contains(where: {
            $0[kAXRoleAttribute] == "AXStaticText" && $0["captureWorkspace"] == "true" &&
                $0["captureParent"] == parent && $0["captureChildIndex"] == String(position + 1) &&
                $0[kAXValueAttribute] == version
        }) { return true }
    }
    return false
}
func isButton(_ row: [String: String], _ title: String) -> Bool {
    row[kAXRoleAttribute] == kAXButtonRole &&
        [row[kAXTitleAttribute], row[kAXDescriptionAttribute]].contains(title)
}
func nativeError(_ rows: [[String: String]]) -> Bool {
    let errors = ["Local index status and database counts could not be read.",
        "Local index status could not be read.", "Database counts could not be read.",
        "Dashboard metrics could not be loaded.", "The local index has stopped."]
    return rows.contains { row in
        row[kAXRoleAttribute] == "AXAlert" || row[kAXSubroleAttribute] == "AXDialog" ||
        row.values.contains { text in errors.contains { text.contains($0) } }
    }
}
func updateDashboard(_ rows: [[String: String]], _ version: String) -> Bool {
    !nativeError(rows) && rows.contains { $0[kAXRoleAttribute] == kAXWindowRole } &&
        rows.contains { $0[kAXRoleAttribute] == "AXWebArea" } &&
        displayedVersion(rows, version) && !hasText(rows, "Welcome to XTrace") &&
        hasText(rows, "What your agents did") &&
        hasText(rows, "No agent activity was recorded in this range. Measured zeros are shown as 0.")
}
func updateReady(_ rows: [[String: String]]) -> Bool {
    updateDashboard(rows, "0.1.5") &&
        hasText(rows, "Update 0.1.6 is ready. Your local data stays on this Mac.") &&
        rows.filter { isButton($0, "Restart to update") }.count == 1
}
// END UPDATE UI POLICY

let root = AXUIElementCreateApplication(pid)
AXUIElementSetMessagingTimeout(root, 0.5)
let deadline = Date().addingTimeInterval(mode == "update-ready" ? 180 : mode == "update-press" ? 10 : releaseMode ? 90 : 25)
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
        if updateMode {
            if nativeError(last) { finish(6, "Released app displayed an error or dialog", last) }
            if mode == "updated-dashboard" {
                if updateDashboard(last, "0.1.6") {
                    finish(0, "Observed usable native 0.1.6 Dashboard and retained completed Welcome", last)
                }
            } else if updateReady(last) {
                if mode == "update-ready" {
                    finish(0, "Observed usable 0.1.5 Dashboard and target 0.1.6 ready", last)
                }
                let restart = elements.filter { isButton($0.1, "Restart to update") }
                guard restart.count == 1, ownedApplication() != nil else { finish(5, "Identity or restart control changed", last) }
                guard AXUIElementPerformAction(restart[0].0, kAXPressAction as CFString) == .success else {
                    finish(6, "Restart to update AXPress failed", last)
                }
                finish(0, "Pressed real Restart to update once for target 0.1.6", last)
            } else if mode == "update-ready", sawWindow, sawWebContent, !pressed,
                      hasText("Welcome to XTrace"), hasText("The initial scan finished."),
                      hasText("Bundled readers verified"), displayedVersion(last, "0.1.5"),
                      hasText("In the local database now"),
                      !last.contains(where: { $0.values.contains(where: { $0.contains("Reading database counts") }) }) {
                let buttons = elements.filter { isButton($0.1, "Open Dashboard") }
                guard buttons.count == 1, ownedApplication() != nil else { finish(5, "Welcome action identity changed", last) }
                welcomeEvidence = last
                guard AXUIElementPerformAction(buttons[0].0, kAXPressAction as CFString) == .success else {
                    finish(6, "Open Dashboard AXPress failed", last)
                }
                pressed = true
            }
        } else if releaseMode {
            let errors = ["Local index status and database counts could not be read.",
                "Local index status could not be read.", "Database counts could not be read.",
                "Dashboard metrics could not be loaded.", "The local index has stopped."]
            if last.contains(where: { row in
                row[kAXRoleAttribute] == "AXAlert" || row[kAXSubroleAttribute] == "AXDialog" ||
                    row.values.contains(where: { text in errors.contains(where: { text.contains($0) }) })
            }) { finish(6, "The released app displayed a startup or native-data error", last) }
            if sawWindow, sawWebContent, !pressed, let button,
               hasText("Welcome to XTrace"), hasText("The initial scan finished."),
               hasText("Bundled readers verified"), displayedVersion(last, "0.1.6"),
               hasText("In the local database now"),
               !last.contains(where: { $0.values.contains(where: { $0.contains("Reading database counts") }) }) {
                guard ownedApplication() != nil else { finish(5, "Identity changed before press", last) }
                welcomeEvidence = last
                guard AXUIElementPerformAction(button, kAXPressAction as CFString) == .success else {
                    finish(6, "Open Dashboard AXPress failed", last)
                }
                pressed = true
            } else if sawWindow, sawWebContent, pressed, displayedVersion(last, "0.1.6"),
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
