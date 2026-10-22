import AppKit
import ApplicationServices

// External, read/press-only Accessibility driver. No injected JS, product IPC,
// TCC changes, permission prompts or app launch/termination commands.
var telemetryEvidence: [String: Any]?
func finish(_ code: Int32, _ reason: String, _ nodes: [[String: String]] = []) -> Never {
    var report: [String: Any] = ["code": code, "reason": reason, "nodes": nodes]
    if let telemetryEvidence { report["telemetry"] = telemetryEvidence }
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
let updateMode = ["update-ready", "update-press", "updated-dashboard", "reopened-dashboard"].contains(mode)
let telemetryModes = ["telemetry-notice", "telemetry-turn-off", "telemetry-dashboard", "telemetry-settings-off", "telemetry-settings-on", "telemetry-settings-enable", "telemetry-settings-disable", "telemetry-environment-off"]
let telemetryMode = telemetryModes.contains(mode)
let releaseMode = mode == "first-launch" || updateMode || telemetryMode
let authorization = ProcessInfo.processInfo.environment
let releaseAuthorized = releaseMode && identifier == "ai.xtrace.app" &&
    bundle.path == Release.bundle(mode) &&
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
        if telemetryMode {
            if let enabled = attribute(element, kAXEnabledAttribute) as? NSNumber { row["captureEnabled"] = enabled.boolValue ? "true" : "false" }
            if ["AXCheckBox", "AXSwitch"].contains(row[kAXRoleAttribute] ?? ""), let value = attribute(element, kAXValueAttribute) as? NSNumber { row[kAXValueAttribute] = value.stringValue }
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
// The one place naming the release pair and each mode's bundle folder. The
// update starts from the published earlier version in its earlier folder; the
// updater writes the candidate into that folder, and a normal quit renames it.
enum Release {
    static let candidate = "0.1.8"
    static let earlier = "0.1.7"
    static let candidateBundle = "/Applications/XTrace.app"
    static let earlierBundle = "/Applications/XTrace Desktop.app"
    static func bundle(_ mode: String) -> String {
        ["update-ready", "update-press", "updated-dashboard"].contains(mode) ? earlierBundle : candidateBundle
    }
    static let readyText = "Update \(candidate) is ready. Your local data stays on this Mac."
}
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
func nativeDataError(_ rows: [[String: String]]) -> Bool {
    let errors = ["Local index status and database counts could not be read.",
        "Local index status could not be read.", "Database counts could not be read.",
        "Dashboard metrics could not be loaded.", "The local index has stopped."]
    return rows.contains { row in row.values.contains { text in errors.contains { text.contains($0) } } }
}
func isAlert(_ row: [String: String]) -> Bool {
    row[kAXRoleAttribute] == "AXAlert" ||
        (row[kAXRoleAttribute] == "AXGroup" && row[kAXSubroleAttribute] == "AXApplicationAlert")
}
func nativeError(_ rows: [[String: String]]) -> Bool {
    nativeDataError(rows) || rows.contains { isAlert($0) || $0[kAXSubroleAttribute] == "AXDialog" }
}
func updateDashboard(_ rows: [[String: String]], _ version: String) -> Bool {
    !nativeError(rows) && rows.contains { $0[kAXRoleAttribute] == kAXWindowRole } &&
        rows.contains { $0[kAXRoleAttribute] == "AXWebArea" } &&
        displayedVersion(rows, version) && !hasText(rows, "Welcome to XTrace") &&
        hasText(rows, "What your agents did") &&
        hasText(rows, "No agent activity was recorded in this range. Measured zeros are shown as 0.")
}
func updateReady(_ rows: [[String: String]]) -> Bool {
    updateDashboard(rows, Release.earlier) &&
        hasText(rows, Release.readyText) &&
        rows.filter { isButton($0, "Restart to update") }.count == 1
}
// END UPDATE UI POLICY
// BEGIN TELEMETRY UI POLICY: fixed frozen-product words and scoped controls.
let blockedUpdaterText = "The update could not be checked or downloaded safely. Your app is still usable."
struct TelemetryAlerts { let safe: Bool; let updaterWarning: Bool }
func telemetryAlerts(_ rows: [[String: String]]) -> TelemetryAlerts {
    let rejected = TelemetryAlerts(safe: false, updaterWarning: false)
    let errors = ["Telemetry could not be turned off.", "The telemetry setting could not be read.",
        "The telemetry setting could not be changed.", "Telemetry is off for now, but turning it off could not be saved.",
        "Update installation failed. Retry installation.", "The update was installed, but restart failed. Retry restarting the app."]
    guard !nativeDataError(rows), !rows.contains(where: { row in
        ["AXDialog", "AXSheet"].contains(row[kAXRoleAttribute] ?? "") || row[kAXSubroleAttribute] == "AXDialog" ||
        row.values.contains { text in errors.contains { text.contains($0) } }
    }) else { return rejected }
    let alerts = rows.filter(isAlert)
    if alerts.isEmpty {
        return TelemetryAlerts(safe: !hasText(rows, blockedUpdaterText) && !rows.contains(where: { isButton($0, "Retry update") }), updaterWarning: false)
    }
    guard alerts.count == 1, let alertID = alerts[0]["captureNode"], alerts[0]["captureWorkspace"] == "true" else { return rejected }
    var byID: [String: [String: String]] = [:]
    for row in rows { guard let id = row["captureNode"], byID[id] == nil else { return rejected }; byID[id] = row }
    func ancestors(_ row: [String: String]) -> [[String: String]]? {
        var result: [[String: String]] = [], seen = Set<String>(), current = row["captureParent"] ?? ""
        while !current.isEmpty {
            guard seen.insert(current).inserted, let parent = byID[current] else { return nil }
            result.append(parent); current = parent["captureParent"] ?? ""
        }
        return result
    }
    var ancestry: [String: [[String: String]]] = [:]
    for row in rows { guard let parents = ancestors(row) else { return rejected }; ancestry[row["captureNode"]!] = parents }
    guard ancestry[alertID]!.contains(where: { $0[kAXRoleAttribute] == "AXGroup" && $0[kAXSubroleAttribute] == "AXLandmarkComplementary" && $0[kAXDescriptionAttribute] == "Workspace" }) else { return rejected }
    let subtree = rows.filter { $0["captureNode"] == alertID || ancestry[$0["captureNode"]!]!.contains { $0["captureNode"] == alertID } }
    let subtreeIDs = Set(subtree.compactMap { $0["captureNode"] })
    let texts = subtree.flatMap { row in [kAXTitleAttribute, kAXValueAttribute, kAXDescriptionAttribute, kAXHelpAttribute].compactMap { row[$0] }.filter { !$0.isEmpty } }
    guard texts.contains(blockedUpdaterText), subtree.filter({ isButton($0, "Retry update") }).count == 1,
          rows.filter({ isButton($0, "Retry update") }).count == 1,
          rows.filter({ $0.values.contains(blockedUpdaterText) }).allSatisfy({ subtreeIDs.contains($0["captureNode"]!) }),
          texts.allSatisfy({ $0 == blockedUpdaterText || $0 == "Retry update" }) else { return rejected }
    return TelemetryAlerts(safe: true, updaterWarning: true)
}
func telemetryNotice(_ rows: [[String: String]]) -> Bool {
    ["Anonymous usage data", "Telemetry is on by default. XTrace sends a few anonymous events, such as when the app is opened and how often it is used, so we can count how many people use it.",
     "The app version, the macOS version, which coding agents have history on this Mac (Claude, Codex, Cursor), and a random ID created on this Mac.",
     "Repository names, file paths, prompts or other conversation text, pull-request links, token counts or costs."]
        .allSatisfy { hasText(rows, $0) } && rows.filter { isButton($0, "Turn off") && $0["captureEnabled"] == "true" }.count == 1
}
func telemetrySwitches(_ rows: [[String: String]]) -> [[String: String]] {
    rows.filter { ["AXCheckBox", "AXSwitch"].contains($0[kAXRoleAttribute] ?? "") &&
        [$0[kAXTitleAttribute], $0[kAXDescriptionAttribute]].contains("Send anonymous usage data") }
}
func telemetrySwitchState(_ rows: [[String: String]], on: Bool, enabled: Bool) -> Bool {
    let switches = telemetrySwitches(rows)
    return switches.count == 1 && switches[0][kAXValueAttribute] == (on ? "1" : "0") &&
        switches[0]["captureEnabled"] == (enabled ? "true" : "false") && hasText(rows, "Telemetry")
}
// END TELEMETRY UI POLICY

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
var navigatedSettings = false
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
        if telemetryMode {
            let alerts = telemetryAlerts(last)
            guard alerts.safe else { finish(6, "Telemetry phase displayed an unrelated error or dialog", last) }
            telemetryEvidence = ["expectedUpdaterWarning": alerts.updaterWarning, "samePhaseProxyEvidenceRequired": alerts.updaterWarning]
            let ready = sawWindow && sawWebContent && displayedVersion(last, Release.candidate)
            func pressUnique(_ title: String, workspace: Bool = false) {
                let buttons = elements.filter { isButton($0.1, title) && $0.1["captureEnabled"] == "true" && (!workspace || $0.1["captureWorkspace"] == "true") }
                guard buttons.count == 1, ownedApplication() != nil else { finish(5, "Telemetry action ownership or uniqueness failed", last) }
                guard AXUIElementPerformAction(buttons[0].0, kAXPressAction as CFString) == .success else { finish(6, "Telemetry action was not accepted", last) }
            }
            let complete = { finish(0, "Telemetry UI phase \(mode) observed", last) }
            if mode == "telemetry-notice", ready, hasText("Welcome to XTrace"), hasText("The initial scan finished."), hasText("Bundled readers verified"), telemetryNotice(last) { complete() }
            if mode == "telemetry-turn-off", ready {
                if !pressed, telemetryNotice(last) { pressUnique("Turn off"); pressed = true }
                else if pressed, hasText("Telemetry is off. Nothing is sent. You can turn it back on in Settings."), !last.contains(where: { isButton($0, "Turn off") }) { complete() }
            }
            if mode == "telemetry-dashboard", ready {
                if !pressed, hasText("Welcome to XTrace"), hasText("The initial scan finished."), hasText("Bundled readers verified"), hasText("In the local database now"), !last.contains(where: { $0.values.contains(where: { $0.contains("Reading database counts") }) }) { pressUnique("Open Dashboard"); pressed = true }
                else if !hasText("Welcome to XTrace"), hasText("What your agents did"), hasText("No agent activity was recorded in this range. Measured zeros are shown as 0.") { complete() }
            }
            if mode.hasPrefix("telemetry-settings-") || mode == "telemetry-environment-off", ready, !hasText("Welcome to XTrace") {
                if telemetrySwitches(last).isEmpty, !navigatedSettings { pressUnique("Settings", workspace: true); navigatedSettings = true }
                else if mode == "telemetry-environment-off", telemetrySwitchState(last, on: false, enabled: false),
                        hasText("Off for this launch: XTRACE_TELEMETRY turns it off"),
                        hasText("The XTRACE_TELEMETRY environment variable turns telemetry off for this launch. The switch cannot change that; start the app without it to use your saved choice.") { complete() }
                else if mode == "telemetry-settings-off", telemetrySwitchState(last, on: false, enabled: true), hasText("Off") { complete() }
                else if mode == "telemetry-settings-on", telemetrySwitchState(last, on: true, enabled: true), hasText("On") { complete() }
                else if ["telemetry-settings-enable", "telemetry-settings-disable"].contains(mode) {
                    let desired = mode == "telemetry-settings-enable"
                    if !pressed, telemetrySwitchState(last, on: !desired, enabled: true) {
                        let switches = elements.filter { telemetrySwitches([$0.1]).count == 1 }
                        guard switches.count == 1, ownedApplication() != nil else { finish(5, "Telemetry switch identity changed", last) }
                        guard AXUIElementPerformAction(switches[0].0, kAXPressAction as CFString) == .success else { finish(6, "Telemetry switch action failed", last) }
                        pressed = true
                    } else if pressed, telemetrySwitchState(last, on: desired, enabled: true), hasText(desired ? "On" : "Off") { complete() }
                }
            }
        } else if updateMode {
            if nativeError(last) { finish(6, "Released app displayed an error or dialog", last) }
            if mode == "updated-dashboard" || mode == "reopened-dashboard" {
                // Completed Welcome must survive the update and the rename; the
                // start page never shows Welcome first when it was completed.
                if sawWebContent && hasText("Welcome to XTrace") {
                    finish(6, "Welcome was shown again after the update", last)
                }
                if updateDashboard(last, Release.candidate) {
                    finish(0, mode == "updated-dashboard"
                        ? "Observed usable native \(Release.candidate) Dashboard after restart and retained completed Welcome"
                        : "Observed usable native \(Release.candidate) Dashboard after reopening the renamed app without Welcome", last)
                }
            } else if updateReady(last) {
                if mode == "update-ready" {
                    finish(0, "Observed usable \(Release.earlier) Dashboard and target \(Release.candidate) ready", last)
                }
                let restart = elements.filter { isButton($0.1, "Restart to update") }
                guard restart.count == 1, ownedApplication() != nil else { finish(5, "Identity or restart control changed", last) }
                guard AXUIElementPerformAction(restart[0].0, kAXPressAction as CFString) == .success else {
                    finish(6, "Restart to update AXPress failed", last)
                }
                finish(0, "Pressed real Restart to update once for target \(Release.candidate)", last)
            } else if mode == "update-ready", sawWindow, sawWebContent, !pressed,
                      hasText("Welcome to XTrace"), hasText("The initial scan finished."),
                      hasText("Bundled readers verified"), displayedVersion(last, Release.earlier),
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
            if nativeError(last) { finish(6, "The released app displayed a startup or native-data error", last) }
            if sawWindow, sawWebContent, !pressed, let button,
               hasText("Welcome to XTrace"), hasText("The initial scan finished."),
               hasText("Bundled readers verified"), displayedVersion(last, Release.candidate),
               hasText("In the local database now"),
               !last.contains(where: { $0.values.contains(where: { $0.contains("Reading database counts") }) }) {
                guard ownedApplication() != nil else { finish(5, "Identity changed before press", last) }
                welcomeEvidence = last
                guard AXUIElementPerformAction(button, kAXPressAction as CFString) == .success else {
                    finish(6, "Open Dashboard AXPress failed", last)
                }
                pressed = true
            } else if sawWindow, sawWebContent, pressed, displayedVersion(last, Release.candidate),
                      !hasText("Welcome to XTrace"), hasText("What your agents did"),
                      hasText("No agent activity was recorded in this range. Measured zeros are shown as 0.") {
                finish(0, "Observed native-ready Welcome, Open Dashboard action and usable \(Release.candidate) Dashboard",
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
