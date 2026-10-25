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
