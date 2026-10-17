import AppKit
import Darwin

// This helper only launches the fixed installed bundle through LaunchServices,
// inspects its environment without printing values, or stops its exact receipt.
let bundle = URL(fileURLWithPath: "/Applications/XTrace Desktop.app")
let executable = bundle.appendingPathComponent("Contents/MacOS/xtrace-desktop")
let identity = "ai.xtrace.app"
func fail(_ message: String) -> Never { fputs("\(message)\n", stderr); exit(1) }
let authorization = ProcessInfo.processInfo.environment
guard authorization["ALLOW_RELEASE_IDENTITY"] == "true",
      authorization["GITHUB_ACTIONS"] == "true",
      authorization["RUNNER_ENVIRONMENT"] == "github-hosted",
      authorization["GITHUB_EVENT_NAME"] == "workflow_dispatch",
      ProcessInfo.processInfo.operatingSystemVersion.majorVersion == 14 else {
    fail("Hosted real-identity authorization is missing")
}
#if !arch(arm64)
fail("Requires arm64")
#endif
guard CommandLine.arguments.count == 4,
      ["launch", "inspect", "stop"].contains(CommandLine.arguments[1]) else { fail("Invalid command") }
let command = CommandLine.arguments[1]
let receiptURL = URL(fileURLWithPath: CommandLine.arguments[2])
let environmentURL = URL(fileURLWithPath: CommandLine.arguments[3])

struct Receipt: Codable {
    let pid: Int32
    let launchDate: Double
    let bundle: String
    let executable: String
    let identifier: String
}
@Sendable func canonical(_ url: URL?) -> String? { url?.standardizedFileURL.resolvingSymlinksInPath().path }
@Sendable func hasIdentity(_ app: NSRunningApplication) -> Bool {
    app.bundleIdentifier == identity && canonical(app.bundleURL) == bundle.path &&
        canonical(app.executableURL) == executable.path
}
func owned(_ receipt: Receipt) -> NSRunningApplication? {
    guard receipt.bundle == bundle.path, receipt.executable == executable.path,
          receipt.identifier == identity, receipt.pid > 0 else { fail("Invalid ownership receipt") }
    guard let app = NSRunningApplication(processIdentifier: receipt.pid), !app.isTerminated else { return nil }
    guard hasIdentity(app), app.launchDate?.timeIntervalSince1970 == receipt.launchDate else {
        fail("Process identity or launch date differs from receipt")
    }
    return app
}
func wait(_ seconds: Double, until condition: () -> Bool) -> Bool {
    let deadline = Date().addingTimeInterval(seconds)
    while !condition() && Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.1)) }
    return condition()
}

// KERN_PROCARGS2 returns argc, executable, argv, then NUL-separated environment.
// Values stay in memory; no full command/environment dump enters artifacts.
func processEnvironment(_ pid: Int32) -> [String: String] {
    var mib = [CTL_KERN, KERN_PROCARGS2, pid]
    var count = 0
    guard sysctl(&mib, 3, nil, &count, nil, 0) == 0, count >= 4, count <= 4 * 1024 * 1024 else {
        fail("Cannot size owned process environment")
    }
    var bytes = [UInt8](repeating: 0, count: count)
    guard sysctl(&mib, 3, &bytes, &count, nil, 0) == 0 else { fail("Cannot read owned process environment") }
    bytes = Array(bytes.prefix(count))
    let argc = bytes.withUnsafeBytes { $0.loadUnaligned(as: Int32.self) }
    guard argc > 0, argc < 10000 else { fail("Invalid process argument count") }
    var at = 4
    func string() -> String {
        let start = at
        while at < bytes.count && bytes[at] != 0 { at += 1 }
        guard at < bytes.count else { fail("Truncated process environment") }
        let text = String(decoding: bytes[start..<at], as: UTF8.self)
        at += 1
        return text
    }
    _ = string()
    while at < bytes.count && bytes[at] == 0 { at += 1 }
    for _ in 0..<argc { _ = string() }
    var result: [String: String] = [:]
    while at < bytes.count && bytes[at] != 0 {
        let pair = string().split(separator: "=", maxSplits: 1, omittingEmptySubsequences: false)
        guard pair.count == 2, result[String(pair[0])] == nil else { fail("Invalid or duplicate environment key") }
        result[String(pair[0])] = String(pair[1])
    }
    return result
}

do {
    let env = try JSONDecoder().decode([String: String].self, from: Data(contentsOf: environmentURL))
    if command == "launch" {
        guard NSRunningApplication.runningApplications(withBundleIdentifier: identity).isEmpty,
              NSRunningApplication.runningApplications(withBundleIdentifier: "ai.xtrace.desktop").isEmpty else {
            fail("An XTrace instance already exists")
        }
        guard !FileManager.default.fileExists(atPath: receiptURL.path) else { fail("Launch receipt already exists") }
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.environment = env
        configuration.allowsRunningApplicationSubstitution = false
        configuration.createsNewApplicationInstance = true
        configuration.addsToRecentItems = false
        var done = false
        var launchError: String?
        NSWorkspace.shared.openApplication(at: bundle, configuration: configuration) { app, error in
            DispatchQueue.main.async {
                defer { done = true }
                guard let app, hasIdentity(app), let date = app.launchDate else {
                    launchError = error?.localizedDescription ?? "LaunchServices returned no owned process"
                    return
                }
                do {
                    let receipt = Receipt(pid: app.processIdentifier, launchDate: date.timeIntervalSince1970,
                                          bundle: bundle.path, executable: executable.path, identifier: identity)
                    try JSONEncoder().encode(receipt).write(to: receiptURL, options: .atomic)
                } catch { launchError = "Cannot save launch ownership receipt" }
            }
        }
        guard wait(45, until: { done }) else { fail("LaunchServices timed out; app ownership may be uncertain") }
        if let launchError { fail(launchError) }
        print("LaunchServices returned the owned application; receipt saved")
    } else {
        let receipt = try JSONDecoder().decode(Receipt.self, from: Data(contentsOf: receiptURL))
        if command == "inspect" {
            guard owned(receipt) != nil else { fail("Owned application exited before inspection") }
            let actual = processEnvironment(receipt.pid)
            for (key, value) in env where actual[key] != value { fail("Required process environment differs: \(key)") }
            // LaunchServices may add system values. HOME must remain the real home.
            if let home = actual["HOME"], home != NSHomeDirectory() { fail("Unexpected HOME override") }
            let systemKeys: Set<String> = ["HOME", "USER", "LOGNAME", "SHELL", "__CF_USER_TEXT_ENCODING",
                "XPC_SERVICE_NAME", "XPC_FLAGS", "COMMAND_MODE", "SECURITYSESSIONID", "__CFBundleIdentifier"]
            for key in actual.keys where env[key] == nil && !systemKeys.contains(key) {
                fail("Unexpected process environment key: \(key)")
            }
            guard owned(receipt) != nil else { fail("Identity lost while reading environment") }
            print("Owned process has required synthetic paths; no unexpected environment keys")
        } else {
            if let app = owned(receipt) {
                _ = app.terminate()
                if !wait(10, until: { app.isTerminated }) {
                    print("Graceful exit did not finish; terminating the exact owned process")
                    if let stillOwned = owned(receipt) { _ = stillOwned.forceTerminate() }
                    guard wait(5, until: { app.isTerminated }) else { fail("Owned process exit unconfirmed") }
                }
            }
            print("Owned process exit confirmed")
        }
    }
} catch { fail("Launch helper could not read or write its owned files") }
