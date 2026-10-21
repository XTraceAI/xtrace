import AppKit
import ApplicationServices
import Darwin

// Hosted test only: observe one attributable SecurityAgent dialog and press Cancel.
// No credentials, keyboard events, approval actions or permission-setting changes.
let securityExecutable = "/System/Library/Frameworks/Security.framework/Versions/A/MachServices/SecurityAgent.bundle/Contents/MacOS/SecurityAgent"
var evidence: [String: Any] = ["status": "failed", "cancelPressed": false]
func finish(_ reason: String, _ passed: Bool = false) -> Never {
    evidence["reason"] = reason
    evidence["status"] = passed ? "passed" : "failed"
    if let bytes = try? JSONSerialization.data(withJSONObject: evidence, options: [.prettyPrinted, .sortedKeys]) {
        FileHandle.standardOutput.write(bytes); print("")
    }
    exit(passed ? 0 : 1)
}
let args = CommandLine.arguments
let env = ProcessInfo.processInfo.environment
guard args.count == 5, env["GITHUB_ACTIONS"] == "true",
      env["RUNNER_ENVIRONMENT"] == "github-hosted",
      env["GITHUB_EVENT_NAME"] == "workflow_dispatch",
      ProcessInfo.processInfo.operatingSystemVersion.majorVersion == 14,
      getuid() != 0, AXIsProcessTrusted() else { finish("Hosted non-root AX prerequisite unavailable") }
#if !arch(arm64)
finish("Requires arm64")
#endif
let root = URL(fileURLWithPath: args[1]).standardizedFileURL.resolvingSymlinksInPath()
let helperName = args[2]
let requesterPath = URL(fileURLWithPath: args[3]).standardizedFileURL.resolvingSymlinksInPath().path
let mode = args[4]
guard root.lastPathComponent == "staging.noindex",
      helperName.range(of: #"^XTraceUpdaterTest-[a-f0-9]{32}$"#, options: .regularExpression) != nil,
      requesterPath.hasPrefix(root.path + "/"),
      requesterPath.hasSuffix("/Contents/MacOS/" + helperName),
      ["observe", "absent"].contains(mode) else { finish("Invalid owned observer arguments") }

struct Identity: Equatable {
    let pid: pid_t
    let uid: uid_t
    let start: UInt64
    let path: String
    var json: [String: Any] { ["pid": pid, "uid": uid, "startMicros": start, "path": path] }
}
func identity(_ pid: pid_t) -> Identity? {
    var info = proc_bsdinfo()
    guard proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &info, Int32(MemoryLayout<proc_bsdinfo>.size)) ==
          MemoryLayout<proc_bsdinfo>.size, info.pbi_pid == UInt32(pid), info.pbi_start_tvsec > 0 else { return nil }
    var bytes = [CChar](repeating: 0, count: 4 * Int(MAXPATHLEN))
    guard proc_pidpath(pid, &bytes, UInt32(bytes.count)) > 0 else { return nil }
    return Identity(pid: pid, uid: info.pbi_uid,
                    start: info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec,
                    path: String(cString: bytes))
}
func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success else { return nil }
    return value
}
typealias Node = (element: AXUIElement, row: [String: String])
func snapshot(_ window: AXUIElement) -> [Node]? {
    var queue: [(AXUIElement, Int, Int)] = [(window, 0, -1)], result: [Node] = []
    while result.count < queue.count {
        let index = result.count
        guard index < 500 else { return nil }
        let (element, depth, parent) = queue[index]
        var row = ["node": String(index), "parent": String(parent)]
        for key in [kAXRoleAttribute, kAXSubroleAttribute, kAXTitleAttribute] {
            if let value = attribute(element, key) as? String { row[key] = value }
        }
        // Never capture editable/password field values.
        if row[kAXRoleAttribute] == kAXStaticTextRole,
           let value = attribute(element, kAXValueAttribute) as? String { row[kAXValueAttribute] = value }
        if let enabled = attribute(element, kAXEnabledAttribute) as? NSNumber {
            row["enabled"] = enabled.boolValue ? "true" : "false"
        }
        result.append((element, row))
        if let children = attribute(element, kAXChildrenAttribute) as? [AXUIElement], !children.isEmpty {
            guard depth < 15, queue.count + children.count <= 500 else { return nil }
            queue.append(contentsOf: children.map { ($0, depth + 1, index) })
        }
    }
    return result
}
struct Capture {
    let owner: Identity
    let window: AXUIElement
    let nodes: [Node]
    var json: [String: Any] { ["owner": owner.json, "nodes": nodes.map(\.row)] }
}
func capture() -> [Capture]? {
    var result: [Capture] = []
    for app in NSWorkspace.shared.runningApplications where app.bundleIdentifier == "com.apple.SecurityAgent" {
        guard let owner = identity(app.processIdentifier), owner.path == securityExecutable else { return nil }
        let application = AXUIElementCreateApplication(owner.pid)
        guard let windows = attribute(application, kAXWindowsAttribute) as? [AXUIElement] else { return nil }
        for window in windows {
            guard let nodes = snapshot(window) else { return nil }
            result.append(Capture(owner: owner, window: window, nodes: nodes))
        }
        evidence["securityAgentWithoutWindows"] = windows.isEmpty ? owner.json : NSNull()
    }
    return result
}
// BEGIN CANCEL POLICY: also compiled with synthetic rows, without AppKit or AX.
func cancelIndex(_ rows: [[String: String]], _ helperName: String) -> Int? {
    // Hosted macOS 14 SecurityAgent reports its password prompt as an
    // "Untitled" AXStandardWindow (run 39521672625), not an AXDialog.
    guard rows.first?["AXRole"] == "AXWindow",
          ["AXDialog", "AXStandardWindow"].contains(rows.first?["AXSubrole"] ?? "") else { return nil }
    let pattern = #"(?<![A-Za-z0-9_-])"# + NSRegularExpression.escapedPattern(for: helperName) + #"(?![A-Za-z0-9_-])"#
    guard rows.contains(where: { row in
        row["AXRole"] == "AXStaticText" &&
        (row["AXValue"] ?? "").range(of: pattern, options: .regularExpression) != nil
    }) else { return nil }
    let controls = rows.indices.filter {
        rows[$0]["AXRole"] == "AXButton" &&
        rows[$0]["AXTitle"] == "Cancel" && rows[$0]["enabled"] == "true"
    }
    return controls.count == 1 ? controls[0] : nil
}
// END CANCEL POLICY
func cancelControl(_ captured: Capture) -> AXUIElement? {
    guard let index = cancelIndex(captured.nodes.map(\.row), helperName) else { return nil }
    return captured.nodes[index].element
}
// BEGIN REFRESH WAIT: Foundation-only controls exercise this exact wait.
func waitForRefresh(until deadline: Date) {
    let sliceEnd = min(deadline, Date().addingTimeInterval(0.1))
    guard sliceEnd.timeIntervalSinceNow > 0 else { return }
    // The default mode belongs to the common set. NSWorkspace app-list changes
    // require a main-run-loop turn; .common is not itself a runnable mode.
    _ = RunLoop.main.run(mode: .default, before: sliceEnd)
    // An empty run loop can return immediately. Keep the polling slice bounded
    // without spinning or moving the caller's absolute deadline.
    if sliceEnd.timeIntervalSinceNow > 0 { Thread.sleep(until: sliceEnd) }
}
// END REFRESH WAIT
guard let baseline = capture() else { finish("Cannot establish SecurityAgent baseline") }
evidence["baseline"] = baseline.map(\.json)
guard baseline.isEmpty else { finish("Pre-existing authorization window; no action taken") }
if mode == "absent" { finish("No SecurityAgent windows remain", true) }
do { try Data("ready\n".utf8).write(to: root.appendingPathComponent("observer-ready"), options: .withoutOverwriting) }
catch { finish("Cannot write unique observer readiness receipt") }
let deadline = Date().addingTimeInterval(40)
while Date() < deadline {
    guard let captures = capture() else { finish("SecurityAgent ownership/capture unavailable") }
    if !captures.isEmpty {
        evidence["before"] = captures.map(\.json)
        guard captures.count == 1, let cancel = cancelControl(captures[0]),
              let attemptData = try? Data(contentsOf: root.appendingPathComponent("permission-attempt.json")),
              let attempt = try? JSONSerialization.jsonObject(with: attemptData) as? [String: Any],
              let pid = attempt["pid"] as? Int32, let requester = identity(pid),
              requester.path == requesterPath, requester.uid == getuid(),
              let receipt = try? Data(contentsOf: root.appendingPathComponent("permission-install.json")),
              let install = try? JSONSerialization.jsonObject(with: receipt) as? [String: Any],
              let installer = install["process"] as? [String: Any],
              installer["pid"] as? Int32 == pid,
              installer["startMicros"] as? UInt64 == requester.start,
              install["installAttempted"] as? Bool == true else { finish("Dialog does not uniquely identify the active owned install") }
        evidence["requester"] = requester.json
        guard let fresh = capture(), fresh.count == 1,
              fresh[0].owner == captures[0].owner,
              CFEqual(fresh[0].window, captures[0].window),
              let freshCancel = cancelControl(fresh[0]), CFEqual(cancel, freshCancel),
              identity(pid) == requester else { finish("Prompt, requester or Cancel identity changed") }
        evidence["action"] = fresh[0].json
        guard AXUIElementPerformAction(freshCancel, kAXPressAction as CFString) == .success else {
            finish("Cancel action failed")
        }
        evidence["cancelPressed"] = true
        let disappearedBy = Date().addingTimeInterval(5)
        while Date() < disappearedBy {
            guard let after = capture() else { finish("Cannot confirm prompt disappearance") }
            evidence["after"] = after.map(\.json)
            if after.isEmpty { finish("Canceled the uniquely identified owned authorization dialog", true) }
            waitForRefresh(until: disappearedBy)
        }
        finish("Authorization window remained after Cancel")
    }
    waitForRefresh(until: deadline)
}
finish("No attributable cancelable prompt observed within 40 seconds")
