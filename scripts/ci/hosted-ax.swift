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
let telemetryModes = ["telemetry-notice", "telemetry-dashboard-notice", "telemetry-turn-off", "telemetry-settings-off", "telemetry-settings-on", "telemetry-settings-enable", "telemetry-settings-disable", "telemetry-environment-off"]
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

// __hosted-ax-update-ui-policy.swift__
// __hosted-ax-telemetry-ui-policy.swift__

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
            let complete = { finish(0, telemetryReason(mode), last) }
            if mode == "telemetry-notice", ready, hasText("Welcome to XTrace"), hasText("The initial scan finished."), hasText("Bundled readers verified"), telemetryNotice(last) { complete() }
            let step = mode == "telemetry-dashboard-notice" ? telemetryDashboardNoticeStep(last, pressed: pressed) :
                mode == "telemetry-turn-off" ? telemetryTurnOffStep(last, pressed: pressed) : TelemetryStep.wait
            if ready, case .press(let title) = step { pressUnique(title); pressed = true }
            else if ready, step == .done { complete() }
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
                // Completed Welcome must survive the update and reopening; the
                // start page never shows Welcome first when it was completed.
                if sawWebContent && hasText("Welcome to XTrace") {
                    finish(6, "Welcome was shown again after the update", last)
                }
                if updateDashboard(last, Release.candidate) {
                    finish(0, mode == "updated-dashboard"
                        ? "Observed usable native \(Release.candidate) Dashboard after restart and retained completed Welcome"
                        : "Observed usable native \(Release.candidate) Dashboard after reopening \(URL(fileURLWithPath: Release.candidateBundle).lastPathComponent) without Welcome", last)
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
