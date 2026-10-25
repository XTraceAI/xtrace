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
