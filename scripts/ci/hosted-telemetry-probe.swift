import AppKit
import Darwin
// __TELEMETRY_ENVIRONMENT_POLICY__

// Harmless hosted-only fixture: validates inheritance, runs fixed .invalid curl,
// then remains available for the same receipt-based stop used by the product.
let app = NSApplication.shared
app.setActivationPolicy(.prohibited)
let timer = Timer.scheduledTimer(withTimeInterval: 60, repeats: false) { _ in app.terminate(nil) }
DispatchQueue.main.async {
    var report: [String: Any] = ["passed": false, "pid": getpid(), "curlExitConfirmed": false]
    var ownedResult: URL?
    var ownedCurl: Process?
    let finished = DispatchSemaphore(value: 0)
    do {
        guard CommandLine.arguments.count == 3 else { throw ContextFailure.invalid }
        let environment = URL(fileURLWithPath: CommandLine.arguments[1])
        let result = URL(fileURLWithPath: CommandLine.arguments[2])
        guard environment.deletingLastPathComponent() == result.deletingLastPathComponent(),
              environment.deletingLastPathComponent().lastPathComponent == "staging.noindex",
              environment.standardizedFileURL.resolvingSymlinksInPath() == environment,
              result.standardizedFileURL.resolvingSymlinksInPath() == result,
              result.lastPathComponent.hasPrefix("probe-receipt-"), result.lastPathComponent.hasSuffix(".json.result.json"),
              !FileManager.default.fileExists(atPath: result.path),
              Bundle.main.bundleIdentifier == "ai.xtrace.app.test.telemetry-proxy" else { throw ContextFailure.invalid }
        ownedResult = result
        let expected = try JSONDecoder().decode([String: String].self, from: Data(contentsOf: environment))
        guard let home = genuineHome(), telemetryEnvironmentMatches(ProcessInfo.processInfo.environment, expected: expected,
            owned: environment.deletingLastPathComponent(), home: home) else { throw ContextFailure.invalid }
        report["environmentMatches"] = true
        report["genuineHome"] = true
        let child = Process()
        child.executableURL = URL(fileURLWithPath: "/usr/bin/curl")
        child.arguments = __CURL_ARGUMENTS__
        // Do not set child.environment: curl inherits the actual verified environment.
        let input = Pipe(), output = Pipe()
        child.standardInput = input; child.standardOutput = output
        child.standardError = FileHandle.nullDevice
        child.terminationHandler = { _ in finished.signal() }
        try child.run()
        ownedCurl = child
        try input.fileHandleForWriting.write(contentsOf: Data("{\"probe\":true}".utf8))
        try input.fileHandleForWriting.close()
        guard finished.wait(timeout: .now() + 17) == .success else { throw ContextFailure.invalid }
        child.waitUntilExit()
        report["curlExitConfirmed"] = true
        let bytes = output.fileHandleForReading.readDataToEndOfFile()
        guard bytes.count <= 16 else { throw ContextFailure.invalid }
        report["curlStatus"] = child.terminationStatus
        report["httpCode"] = String(decoding: bytes, as: UTF8.self)
        report["passed"] = child.terminationReason == .exit && child.terminationStatus == 56 && bytes == Data("000".utf8)
        try JSONSerialization.data(withJSONObject: report, options: [.sortedKeys]).write(to: result, options: .withoutOverwriting)
    } catch {
        report["error"] = "Probe environment, bounded curl, or evidence check failed"
        if let child = ownedCurl {
            if child.isRunning {
                child.terminate()
                if finished.wait(timeout: .now() + 2) != .success, child.isRunning {
                    kill(child.processIdentifier, SIGKILL) // Only this unreaped Process child.
                    _ = finished.wait(timeout: .now() + 2)
                }
            }
            if !child.isRunning { child.waitUntilExit(); report["curlExitConfirmed"] = true }
        }
        if let result = ownedResult {
            try? JSONSerialization.data(withJSONObject: report, options: [.sortedKeys]).write(to: result, options: .withoutOverwriting)
        }
    }
}
enum ContextFailure: Error { case invalid }
app.run()
