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
// BEGIN LAUNCH CONTEXT POLICY: fixed product and fixed owned telemetry probe only.
// `earlier-` commands act on the folder releases through 0.1.7 installed under
// (the update starts there and the updater writes the new version into it);
// all other product commands act on the current folder. Arm/adopt exist only
// for the earlier folder, the one that is updated in place.
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
// END LAUNCH CONTEXT POLICY
let context: LaunchContext
do { context = try launchContext(CommandLine.arguments[1], environment: environmentURL, receipt: receiptURL) }
catch { fail("Unsupported command or owned telemetry path") }
let command = context.command
let bundle = context.bundle
let executable = context.executable
let identity = context.identifier

// BEGIN TELEMETRY ENVIRONMENT POLICY: also compiled into the harmless probe.
let telemetrySystemKeys: Set<String> = ["HOME", "USER", "LOGNAME", "SHELL", "__CF_USER_TEXT_ENCODING",
    "XPC_SERVICE_NAME", "XPC_FLAGS", "COMMAND_MODE", "SECURITYSESSIONID", "__CFBundleIdentifier"]
func genuineHome() -> String? {
    guard let entry = getpwuid(getuid()), let home = entry.pointee.pw_dir else { return nil }
    return String(cString: home)
}
func telemetryEnvironmentMatches(_ actual: [String: String], expected: [String: String], owned: URL, home: String) -> Bool {
    let required: Set<String> = ["PATH", "LANG", "HOME", "TMPDIR", "XTRACE_NATIVE_HOME", "CLAUDE_CONFIG_DIR", "GH_CONFIG_DIR", "SSH_AUTH_SOCK", "HTTPS_PROXY", "https_proxy", "NO_PROXY", "no_proxy"]
    guard Set(expected.keys) == required || Set(expected.keys) == required.union(["XTRACE_TELEMETRY"]),
          expected["XTRACE_TELEMETRY"] == nil || expected["XTRACE_TELEMETRY"] == "0",
          expected["PATH"] == "/usr/bin:/bin:/usr/sbin:/sbin", expected["LANG"] == "en_US.UTF-8",
          expected["HOME"] == home, expected["SSH_AUTH_SOCK"] == "",
          expected["NO_PROXY"] == "", expected["no_proxy"] == "",
          let proxy = expected["HTTPS_PROXY"], proxy == expected["https_proxy"],
          proxy.range(of: #"^http://127\.0\.0\.1:[1-9][0-9]{0,4}$"#, options: .regularExpression) != nil,
          let port = Int(proxy.split(separator: ":").last ?? ""), port <= 65535 else { return false }
    for (key, suffix) in [("TMPDIR", "tmp"), ("XTRACE_NATIVE_HOME", "home"), ("CLAUDE_CONFIG_DIR", "claude"), ("GH_CONFIG_DIR", "gh")] {
        if expected[key] != owned.appendingPathComponent(suffix).path { return false }
    }
    guard expected.allSatisfy({ actual[$0.key] == $0.value }), Set(actual.keys).isSubset(of: Set(expected.keys).union(telemetrySystemKeys)) else { return false }
    return actual["HOME"] == home
}
func telemetryHelperEnvironmentIsSafe(_ actual: [String: String], home: String) -> Bool {
    actual["PATH"] == "/usr/bin:/bin:/usr/sbin:/sbin" && actual["LANG"] == "en_US.UTF-8" &&
        (actual["HOME"] == nil || actual["HOME"] == home) &&
        Set(actual.keys).isSubset(of: telemetrySystemKeys.union(["PATH", "LANG"]))
}
// END TELEMETRY ENVIRONMENT POLICY
func launchConfiguration(_ environment: [String: String]) -> NSWorkspace.OpenConfiguration {
    let configuration = NSWorkspace.OpenConfiguration()
    configuration.environment = environment
    configuration.allowsRunningApplicationSubstitution = false
    configuration.createsNewApplicationInstance = true
    configuration.addsToRecentItems = false
    return configuration
}

struct Receipt: Codable {
    let pid: Int32
    let startTime: UInt64
    let bundle: String
    let executable: String
    let identifier: String
}
// The installed app's two fixed bundle folders: "XTrace Desktop.app" through
// release 0.1.7, "XTrace.app" since (an updated earlier copy renames itself on
// a normal quit). Static, so it is ready wherever top-level code reads it.
enum ProductBundle {
    static let earlier = "/Applications/XTrace Desktop.app"
    static let current = "/Applications/XTrace.app"
}
// BEGIN TRANSITION POLICY: compiled directly by offline value-only controls.
struct Transition: Codable {
    let original: Receipt
    let armedAt: UInt64
}
enum TransitionError: Error { case unexpectedIdentity }
func successor(_ transition: Transition, _ candidates: [Receipt], originalAlive: Bool) throws -> Receipt? {
    let old = transition.original
    func fixed(_ candidate: Receipt) -> Bool {
        candidate.pid > 0 && candidate.bundle == ProductBundle.earlier &&
        candidate.executable == ProductBundle.earlier + "/Contents/MacOS/xtrace-desktop" &&
        candidate.identifier == "ai.xtrace.app"
    }
    guard fixed(old), old.startTime > 0, transition.armedAt >= old.startTime else { throw TransitionError.unexpectedIdentity }
    guard Set(candidates.map { $0.pid }).count == candidates.count else { throw TransitionError.unexpectedIdentity }
    var replacements: [Receipt] = []
    for candidate in candidates {
        guard fixed(candidate), candidate.startTime > 0 else { throw TransitionError.unexpectedIdentity }
        if candidate.pid == old.pid {
            guard originalAlive, candidate.startTime == old.startTime else { throw TransitionError.unexpectedIdentity }
        } else {
            guard candidate.startTime >= transition.armedAt else { throw TransitionError.unexpectedIdentity }
            replacements.append(candidate)
        }
    }
    guard replacements.count <= 1 else { throw TransitionError.unexpectedIdentity }
    return originalAlive ? nil : replacements.first
}
func liveCandidateStart(_ observed: UInt64?, missingProcess: Bool) throws -> UInt64? {
    if let observed {
        guard observed > 0 else { throw TransitionError.unexpectedIdentity }
        return observed
    }
    guard missingProcess else { throw TransitionError.unexpectedIdentity }
    return nil
}
func kernelIsAlive(expected: UInt64, observed: UInt64?, missingProcess: Bool) throws -> Bool {
    guard expected > 0 else { throw TransitionError.unexpectedIdentity }
    guard let live = try liveCandidateStart(observed, missingProcess: missingProcess) else { return false }
    guard live == expected else { throw TransitionError.unexpectedIdentity }
    return true
}
// END TRANSITION POLICY
// BEGIN STOP POLICY: shared query classification and bounded post-request waiting.
enum ProcessMetadata: Equatable { case start(UInt64), unavailable, invalid }
struct ProcessRead {
    let metadata: ProcessMetadata
    let bytes: Int32
    let queryError: Int32
    let killResult: Int32?
    let killError: Int32?
    var start: UInt64? { if case let .start(value) = metadata { return value }; return nil }
    var missing: Bool { start == nil && killResult == -1 && killError == ESRCH }
}
func classifyProcess(_ info: proc_bsdinfo, pid: Int32, uid: uid_t, bytes: Int32) -> ProcessMetadata {
    guard bytes == Int32(MemoryLayout<proc_bsdinfo>.stride) else { return .unavailable }
    guard info.pbi_pid == UInt32(pid), info.pbi_uid == uid, info.pbi_start_tvsec > 0,
          info.pbi_start_tvsec < UInt64.max / 1_000_000, info.pbi_start_tvusec < 1_000_000 else { return .invalid }
    return .start(info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec)
}
func strictKernelIsAlive(_ read: ProcessRead, expected: UInt64) throws -> Bool {
    guard read.metadata != .invalid else { throw TransitionError.unexpectedIdentity }
    return try kernelIsAlive(expected: expected, observed: read.start, missingProcess: read.missing)
}
enum ExitObservation: Equatable { case matching, pending, absent }
enum StopError: Error { case invalidIdentity, unavailable, deadline }
func exitObservation(_ read: ProcessRead, expected: UInt64) throws -> ExitObservation {
    guard expected > 0, read.metadata != .invalid else { throw StopError.invalidIdentity }
    if let start = read.start {
        guard start == expected else { throw StopError.invalidIdentity }
        return .matching
    }
    guard read.queryError != EPERM, read.queryError != EACCES else { throw StopError.unavailable }
    if read.missing { return .absent }
    // Only the observed exited-but-unreaped case may remain pending. No other
    // query/permission failure becomes life, absence, or permission to signal.
    guard read.bytes == 0, read.queryError == ESRCH, read.killResult == 0 else { throw StopError.unavailable }
    return .pending
}
func waitForExit(_ seconds: Double, observe: () throws -> ExitObservation,
                 now: () -> Double, pause: (Double) -> Void) throws -> Bool {
    let deadline = now() + seconds
    while now() < deadline {
        if try observe() == .absent { return true }
        pause(min(0.1, max(0, deadline - now())))
    }
    // Check the final pause without rereading an absence already confirmed above.
    return try observe() == .absent
}
func stopOwnedProcess(owner: () throws -> Bool, request: (Bool) -> Void,
                      observe: () throws -> ExitObservation, now: () -> Double,
                      pause: (Double) -> Void, phase: (String) -> Void) throws {
    phase("initial-ownership")
    guard try owner() else { return }
    phase("stop-request")
    request(false)
    phase("graceful-wait")
    if try waitForExit(10, observe: observe, now: now, pause: pause) { return }
    phase("pre-force-ownership")
    guard try owner() else { return }
    phase("force-request")
    request(true)
    phase("forced-wait")
    guard try waitForExit(5, observe: observe, now: now, pause: pause) else { throw StopError.deadline }
}
// END STOP POLICY

let transitionURL = URL(fileURLWithPath: receiptURL.path + ".transition.json")
let successorURL = URL(fileURLWithPath: receiptURL.path + ".successor.json")
// AppKit launchDate is absent for direct-spawn restarts. Kernel birth time is
// required for every receipt, including the initial LaunchServices process.
// BEGIN PROCESS QUERY: actual owned-child tests compile this unchanged adapter.
func readProcess(_ pid: Int32) -> ProcessRead {
    var info = proc_bsdinfo()
    let size = Int32(MemoryLayout<proc_bsdinfo>.stride)
    errno = 0
    let bytes = proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &info, size)
    let queryError = errno
    let metadata = classifyProcess(info, pid: pid, uid: getuid(), bytes: bytes)
    if case .start = metadata {
        return ProcessRead(metadata: metadata, bytes: bytes, queryError: queryError, killResult: nil, killError: nil)
    }
    errno = 0
    let result = kill(pid, 0)
    return ProcessRead(metadata: metadata, bytes: bytes, queryError: queryError, killResult: result, killError: errno)
}
// END PROCESS QUERY
func processStart(_ pid: Int32) -> UInt64? {
    let read = readProcess(pid)
    guard read.metadata != .invalid else { fail("Kernel process identity fields are invalid") }
    return read.start
}
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
