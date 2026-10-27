// `earlier-` commands own the update lifecycle, including arm/adopt. Both
// releases use the same folder; command roles remain distinct.
struct LaunchContext {
    let command: String
    let bundle: URL
    let executable: URL
    let identifier: String
    let probe: Bool
    let telemetry: Bool
}
enum ContextError: Error { case invalid }
func launchContext(_ raw: String, environment: URL, receipt: URL) throws -> LaunchContext {
    let earlierCommands = ["launch", "inspect", "stop", "quit", "arm-update", "adopt-update", "absent"]
    let productCommands = ["launch", "inspect", "stop", "quit", "absent"]
    let lifecycle = ["launch", "inspect", "stop", "absent"]
    let probe = raw.hasPrefix("probe-")
    let telemetry = probe || raw.hasPrefix("telemetry-")
    let earlier = raw.hasPrefix("earlier-")
    let command = telemetry ? String(raw.dropFirst(probe ? 6 : 10)) : earlier ? String(raw.dropFirst(8)) : raw
    guard (telemetry ? lifecycle : earlier ? earlierCommands : productCommands).contains(command) else { throw ContextError.invalid }
    let owned = environment.deletingLastPathComponent()
    if telemetry {
        let phases = ["notice", "saved-off", "environment-off", "saved-on"]
        guard let phase = phases.first(where: { environment.lastPathComponent == "environment-\($0).json" }),
              owned.lastPathComponent == "staging.noindex",
              owned.deletingLastPathComponent().lastPathComponent.hasPrefix("run-"),
              owned.deletingLastPathComponent().deletingLastPathComponent().lastPathComponent == "hosted-telemetry",
              environment.standardizedFileURL.resolvingSymlinksInPath().path == environment.path,
              receipt.standardizedFileURL.resolvingSymlinksInPath().path == receipt.path,
              receipt.lastPathComponent == "\(probe ? "probe" : "launch")-receipt-\(phase).json",
              receipt.deletingLastPathComponent() == (probe ? owned : owned.deletingLastPathComponent().deletingLastPathComponent()) else { throw ContextError.invalid }
    }
    let bundle = probe ? owned.appendingPathComponent("Telemetry Proxy Probe.app") :
        URL(fileURLWithPath: earlier ? ProductBundle.earlier : ProductBundle.current)
    let executable = bundle.appendingPathComponent(probe ? "Contents/MacOS/TelemetryProxyProbe" : "Contents/MacOS/xtrace-desktop")
    return LaunchContext(command: command, bundle: bundle, executable: executable,
                         identifier: probe ? "ai.xtrace.app.test.telemetry-proxy" : "ai.xtrace.app", probe: probe, telemetry: telemetry)
}
func probeMetadataMatches(_ info: [String: Any]) -> Bool {
    info["CFBundleIdentifier"] as? String == "ai.xtrace.app.test.telemetry-proxy" && info["CFBundleExecutable"] as? String == "TelemetryProxyProbe"
}
func receiptMatchesContext(_ receipt: Receipt, _ selected: LaunchContext) -> Bool {
    receipt.pid > 0 && receipt.startTime > 0 && receipt.bundle == selected.bundle.path &&
        receipt.executable == selected.executable.path && receipt.identifier == selected.identifier
}
