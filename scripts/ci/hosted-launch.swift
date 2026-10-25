import AppKit
import Darwin

// This hosted-only helper launches the fixed installed product or the fixed owned
// harmless proxy probe, inspects environment without values, and stops exact receipts.
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
// LaunchServices can inherit the caller's environment in addition to the
// configured app values. Consume these helper-only markers before using it.
for key in ["ALLOW_RELEASE_IDENTITY", "GITHUB_ACTIONS", "RUNNER_ENVIRONMENT", "GITHUB_EVENT_NAME"] {
    guard unsetenv(key) == 0, getenv(key) == nil else { fail("Could not remove helper marker: \(key)") }
}
guard CommandLine.arguments.count == 4 else { fail("Invalid command") }
let environmentURL = URL(fileURLWithPath: CommandLine.arguments[3])
let receiptURL = URL(fileURLWithPath: CommandLine.arguments[2])
// __hosted-launch-launch-context-policy.swift__
let context: LaunchContext
do { context = try launchContext(CommandLine.arguments[1], environment: environmentURL, receipt: receiptURL) }
catch { fail("Unsupported command or owned telemetry path") }
let command = context.command
let bundle = context.bundle
let executable = context.executable
let identity = context.identifier

// __hosted-launch-telemetry-environment-policy.swift__
func launchConfiguration(_ environment: [String: String]) -> NSWorkspace.OpenConfiguration {
    let configuration = NSWorkspace.OpenConfiguration()
    configuration.environment = environment
    configuration.allowsRunningApplicationSubstitution = false
    configuration.createsNewApplicationInstance = true
    configuration.addsToRecentItems = false
    return configuration
}

// __hosted-launch-receipt.swift__
// __hosted-launch-transition-policy.swift__
// __hosted-launch-stop-policy.swift__

let transitionURL = URL(fileURLWithPath: receiptURL.path + ".transition.json")
let successorURL = URL(fileURLWithPath: receiptURL.path + ".successor.json")
// AppKit launchDate is absent for direct-spawn restarts. Kernel birth time is
// required for every receipt, including the initial LaunchServices process.
// __hosted-launch-process-query.swift__
// __hosted-launch-process-start.swift__

var stopPhase = "initial-ownership"
var stopObservations = Set<String>()
func diagnoseStop(_ receipt: Receipt, _ read: ProcessRead) {
    guard command == "stop" else { return }
    let key = "\(stopPhase):\(read.metadata):\(read.bytes):\(read.queryError):\(read.killResult ?? 0):\(read.killError ?? 0)"
    guard stopObservations.count < 8, stopObservations.insert(key).inserted else { return }
    let detail: [String: Any] = ["phase": stopPhase, "pid": receipt.pid, "expectedStart": receipt.startTime,
        "metadata": String(describing: read.metadata), "queryBytes": read.bytes, "queryErrno": read.queryError,
        "killZeroResult": read.killResult.map { $0 as Any } ?? NSNull(),
        "killZeroErrno": read.killError.map { $0 as Any } ?? NSNull()]
    if let data = try? JSONSerialization.data(withJSONObject: detail, options: [.sortedKeys]) {
        FileHandle.standardOutput.write(data); print("")
    }
}
var diagnosedCandidates = Set<Int32>()
func candidates() -> [Receipt] {
    NSWorkspace.shared.runningApplications.filter { !$0.isTerminated &&
        ($0.bundleIdentifier == identity || canonical($0.bundleURL) == bundle.path ||
         canonical($0.executableURL) == executable.path) }.compactMap { app -> Receipt? in
        let read = readProcess(app.processIdentifier)
        guard read.metadata != .invalid else { fail("Candidate kernel process identity fields are invalid") }
        let start = read.start
        let missing = read.missing
        // At most four matched candidate snapshots per invocation; no unrelated
        // processes or environment values enter the diagnostic.
        if command == "adopt-update", diagnosedCandidates.count < 4,
           diagnosedCandidates.insert(app.processIdentifier).inserted {
            let detail: [String: Any] = ["pid": app.processIdentifier,
                "identifier": String((app.bundleIdentifier ?? "<missing>").prefix(512)),
                "bundle": String((canonical(app.bundleURL) ?? "<missing>").prefix(512)),
                "executable": String((canonical(app.executableURL) ?? "<missing>").prefix(512)),
                "finishedLaunching": app.isFinishedLaunching, "appKitLaunchDateAvailable": app.launchDate != nil,
                "kernelStartAvailable": start != nil, "kernelStartMicros": start ?? 0, "kernelProcessMissing": missing]
            if let data = try? JSONSerialization.data(withJSONObject: detail, options: [.sortedKeys]) {
                FileHandle.standardOutput.write(data); print("")
            }
        }
        do {
            // AppKit can retain a terminated process. Only kernel ESRCH permits
            // skipping it; an inaccessible or live unidentifiable PID still fails.
            guard let live = try liveCandidateStart(start, missingProcess: missing) else { return nil }
            guard hasIdentity(app) else { fail("Unexpected update process identity") }
            return Receipt(pid: app.processIdentifier, startTime: live,
                           bundle: bundle.path, executable: executable.path, identifier: identity)
        } catch { fail("Candidate kernel process identity is unavailable") }
    }
}
func requireNoLegacyProcess() {
    guard NSRunningApplication.runningApplications(withBundleIdentifier: "ai.xtrace.desktop").isEmpty else {
        fail("An unexpected legacy XTrace process exists")
    }
}
@Sendable func canonical(_ url: URL?) -> String? { url?.standardizedFileURL.resolvingSymlinksInPath().path }
@Sendable func hasIdentity(_ app: NSRunningApplication) -> Bool {
    app.bundleIdentifier == identity && canonical(app.bundleURL) == bundle.path &&
        canonical(app.executableURL) == executable.path
}
func kernelAlive(_ receipt: Receipt) -> Bool {
    guard receiptMatchesContext(receipt, context) else { fail("Invalid ownership receipt") }
    let read = readProcess(receipt.pid)
    diagnoseStop(receipt, read)
    do { return try strictKernelIsAlive(read, expected: receipt.startTime) }
    catch { fail("Kernel process identity is missing or differs from receipt") }
}
func owned(_ receipt: Receipt) -> NSRunningApplication? {
    guard kernelAlive(receipt) else { return nil }
    guard let app = NSRunningApplication(processIdentifier: receipt.pid), !app.isTerminated,
          hasIdentity(app) else { fail("Live owned kernel process has no matching AppKit identity") }
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
    if context.telemetry {
        guard let home = genuineHome(), telemetryEnvironmentMatches(env, expected: env, owned: environmentURL.deletingLastPathComponent(), home: home),
              telemetryHelperEnvironmentIsSafe(ProcessInfo.processInfo.environment, home: home) else { fail("Telemetry environment failed prelaunch qualification") }
    }
    if command == "launch" {
        if context.probe {
            guard bundle.standardizedFileURL.resolvingSymlinksInPath() == bundle,
                  executable.standardizedFileURL.resolvingSymlinksInPath() == executable,
                  let info = Bundle(url: bundle)?.infoDictionary,
                  probeMetadataMatches(info),
                  NSRunningApplication.runningApplications(withBundleIdentifier: "ai.xtrace.app").isEmpty else { fail("Probe bundle or fresh product baseline differs") }
        }
        guard NSRunningApplication.runningApplications(withBundleIdentifier: identity).isEmpty,
              NSRunningApplication.runningApplications(withBundleIdentifier: "ai.xtrace.desktop").isEmpty else {
            fail("An XTrace instance already exists")
        }
        guard !FileManager.default.fileExists(atPath: receiptURL.path) else { fail("Launch receipt already exists") }
        let configuration = launchConfiguration(env)
        if context.probe {
            configuration.arguments = [environmentURL.path, receiptURL.path + ".result.json"]
        }
        var done = false
        var launchError: String?
        NSWorkspace.shared.openApplication(at: bundle, configuration: configuration) { app, error in
            DispatchQueue.main.async {
                defer { done = true }
                guard let app, hasIdentity(app), let start = processStart(app.processIdentifier) else {
                    launchError = error?.localizedDescription ?? "LaunchServices returned no owned process"
                    return
                }
                do {
                    let receipt = Receipt(pid: app.processIdentifier, startTime: start,
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
        if command == "arm-update" {
            requireNoLegacyProcess()
            guard owned(receipt) != nil, candidates().count == 1,
                  !FileManager.default.fileExists(atPath: transitionURL.path),
                  !FileManager.default.fileExists(atPath: successorURL.path) else { fail("Cannot arm update ownership") }
            let transition = Transition(original: receipt, armedAt: UInt64(Date().timeIntervalSince1970 * 1_000_000))
            try JSONEncoder().encode(transition).write(to: transitionURL, options: .withoutOverwriting)
            print("Update transition armed while original process is owned")
        } else if command == "adopt-update" {
            let transition = try JSONDecoder().decode(Transition.self, from: Data(contentsOf: transitionURL))
            guard transition.original.pid == receipt.pid,
                  transition.original.startTime == receipt.startTime,
                  !FileManager.default.fileExists(atPath: successorURL.path) else { fail("Invalid or already adopted transition") }
            let deadline = Date().addingTimeInterval(55)
            while Date() < deadline {
                requireNoLegacyProcess()
                let oldAlive = kernelAlive(receipt)
                if let next = try successor(transition, candidates(), originalAlive: oldAlive) {
                    // Save ownership before environment/UI checks, so failed checks can still stop it.
                    guard owned(next) != nil, !kernelAlive(receipt) else { fail("Update identity changed during adoption") }
                    try JSONEncoder().encode(next).write(to: successorURL, options: .withoutOverwriting)
                    print("Original exited; distinct successor receipt saved")
                    exit(0)
                }
                RunLoop.current.run(until: Date().addingTimeInterval(0.1))
            }
            fail("Original exit and distinct owned successor were not confirmed; no manual relaunch attempted")
        } else if command == "absent" {
            requireNoLegacyProcess()
            guard !kernelAlive(receipt), candidates().isEmpty else { fail("XTrace process remains after cleanup") }
            print("No current or legacy XTrace process remains")
        } else if command == "quit" {
            // One normal quit request, the Quit Apple event the Dock and Cmd-Q
            // send, so the app runs its own exit path. Never forced: a quit that
            // does not finish fails here and cleanup's stop decides the rest.
            guard let app = owned(receipt) else { fail("Owned application exited before the quit request") }
            print("Owned quit request accepted=\(app.terminate())")
            do {
                guard try waitForExit(20, observe: {
                    try exitObservation(readProcess(receipt.pid), expected: receipt.startTime)
                }, now: { ProcessInfo.processInfo.systemUptime }, pause: { seconds in
                    RunLoop.current.run(until: Date().addingTimeInterval(seconds))
                }) else { fail("The app did not finish a normal quit within 20 seconds; it was not forced") }
            } catch { fail("Owned process identity changed while quitting: \(error)") }
            print("Owned process quit normally")
        } else if command == "inspect" {
            guard owned(receipt) != nil else { fail("Owned application exited before inspection") }
            let actual = processEnvironment(receipt.pid)
            for (key, value) in env where actual[key] != value { fail("Required process environment differs: \(key)") }
            // LaunchServices may add system values. HOME must remain the real home.
            if let home = actual["HOME"], home != NSHomeDirectory() { fail("Unexpected HOME override") }
            let systemKeys: Set<String> = ["HOME", "USER", "LOGNAME", "SHELL", "__CF_USER_TEXT_ENCODING",
                "XPC_SERVICE_NAME", "XPC_FLAGS", "COMMAND_MODE", "SECURITYSESSIONID", "__CFBundleIdentifier"]
            let unexpected = actual.keys.filter { env[$0] == nil && !systemKeys.contains($0) }.sorted()
            if !unexpected.isEmpty {
                fail("Unexpected process environment keys: \(unexpected.joined(separator: ", "))")
            }
            guard owned(receipt) != nil else { fail("Identity lost while reading environment") }
            print("Owned process has required synthetic paths; no unexpected environment keys")
        } else {
            var currentOwner: NSRunningApplication?
            do {
                try stopOwnedProcess(owner: {
                    currentOwner = owned(receipt) // Strict before either signal; unknown still fails.
                    return currentOwner != nil
                }, request: { force in
                    guard let app = currentOwner else { fail("Stop request has no confirmed owner") }
                    let accepted = force ? app.forceTerminate() : app.terminate()
                    print("Owned stop request phase=\(stopPhase) accepted=\(accepted)")
                }, observe: {
                    let read = readProcess(receipt.pid)
                    diagnoseStop(receipt, read)
                    return try exitObservation(read, expected: receipt.startTime)
                }, now: { ProcessInfo.processInfo.systemUptime }, pause: { seconds in
                    RunLoop.current.run(until: Date().addingTimeInterval(seconds))
                }, phase: { phase in
                    stopPhase = phase
                    print("Owned stop phase=\(phase) pid=\(receipt.pid)")
                })
            } catch { fail("Owned process stop failed during \(stopPhase): \(error)") }
            print("Owned process exit confirmed")
        }
    }
} catch { fail("Launch helper could not read or write its owned files") }
