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
// The Dashboard's corner card. The candidate records the disclosure as shown, and
// may then send, only once half of a notice is visible: on the Welcome page
// the pinned bar covers it, so the card is where a first launch records it.
func telemetryCard(_ rows: [[String: String]]) -> Bool {
    ["Anonymous usage data is on by default", "A random ID made on this Mac", "Prompts or conversation text"]
        .allSatisfy { hasText(rows, $0) } &&
        rows.filter { isButton($0, "Turn off") && $0["captureEnabled"] == "true" }.count == 1
}
func telemetryDashboard(_ rows: [[String: String]]) -> Bool {
    !hasText(rows, "Welcome to XTrace") && hasText(rows, "What your agents did")
}
// One step of a phase: press its control, finish, or keep reading. A finish
// needs this run's own press first, so finding the notice is never enough.
enum TelemetryStep: Equatable { case press(String), done, wait }
func telemetryDashboardNoticeStep(_ rows: [[String: String]], pressed: Bool) -> TelemetryStep {
    if !pressed {
        return hasText(rows, "Welcome to XTrace") && hasText(rows, "The initial scan finished.") &&
            hasText(rows, "Bundled readers verified") && hasText(rows, "In the local database now") &&
            !rows.contains(where: { $0.values.contains(where: { $0.contains("Reading database counts") }) }) &&
            telemetryNotice(rows) ? .press("Open Dashboard") : .wait
    }
    return telemetryDashboard(rows) && telemetryCard(rows) ? .done : .wait
}
func telemetryTurnOffStep(_ rows: [[String: String]], pressed: Bool) -> TelemetryStep {
    if !pressed { return telemetryDashboard(rows) && telemetryCard(rows) ? .press("Turn off") : .wait }
    return telemetryDashboard(rows) && hasText(rows, "Anonymous usage data is off") &&
        hasText(rows, "Telemetry is off. Nothing is sent.") &&
        !rows.contains(where: { isButton($0, "Turn off") }) ? .done : .wait
}
// What each phase's success establishes, in the words the runner reports.
func telemetryReason(_ mode: String) -> String {
    switch mode {
    case "telemetry-notice": return "Welcome notice text and one enabled Turn off are in the accessibility tree; visibility was not checked"
    case "telemetry-dashboard-notice": return "Pressed Open Dashboard once; the Dashboard and the corner-card notice with one enabled Turn off are in the accessibility tree"
    case "telemetry-turn-off": return "Pressed Turn off once in the corner card; the card then said telemetry is off and no Turn off remained"
    case "telemetry-settings-off": return "Opened Settings; the telemetry switch is off and enabled"
    case "telemetry-settings-on": return "Opened Settings; the telemetry switch is on and enabled"
    case "telemetry-settings-enable": return "Pressed the enabled telemetry switch once; it is now on"
    case "telemetry-settings-disable": return "Pressed the enabled telemetry switch once; it is now off"
    case "telemetry-environment-off": return "Opened Settings; the telemetry switch is off and disabled, with the environment-off explanation"
    default: return "Unsupported telemetry phase"
    }
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
