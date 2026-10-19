// Compiled with the actual shared helper functions by hosted-stop.test.mjs.
let fullSize = Int32(MemoryLayout<proc_bsdinfo>.stride)
var validInfo = proc_bsdinfo()
validInfo.pbi_pid = 10; validInfo.pbi_uid = 20
validInfo.pbi_start_tvsec = 1; validInfo.pbi_start_tvusec = 2
assert(classifyProcess(validInfo, pid: 10, uid: 20, bytes: fullSize) == .start(1_000_002))
for index in 0..<5 {
    var invalid = validInfo
    switch index {
    case 0: invalid.pbi_pid = 11
    case 1: invalid.pbi_uid = 21
    case 2: invalid.pbi_start_tvsec = 0
    case 3: invalid.pbi_start_tvsec = UInt64.max
    default: invalid.pbi_start_tvusec = 1_000_000
    }
    assert(classifyProcess(invalid, pid: 10, uid: 20, bytes: fullSize) == .invalid)
}
assert(classifyProcess(validInfo, pid: 10, uid: 20, bytes: 0) == .unavailable)
let matching = ProcessRead(metadata: .start(100), bytes: fullSize, queryError: 0, killResult: nil, killError: nil)
let pending = ProcessRead(metadata: .unavailable, bytes: 0, queryError: ESRCH, killResult: 0, killError: 0)
let absent = ProcessRead(metadata: .unavailable, bytes: 0, queryError: ESRCH, killResult: -1, killError: ESRCH)
let changed = ProcessRead(metadata: .start(101), bytes: fullSize, queryError: 0, killResult: nil, killError: nil)
let invalid = ProcessRead(metadata: .invalid, bytes: fullSize, queryError: 0, killResult: -1, killError: ESRCH)
let denied = ProcessRead(metadata: .unavailable, bytes: 0, queryError: EPERM, killResult: -1, killError: EPERM)
assert(try! exitObservation(matching, expected: 100) == .matching)
assert(try! exitObservation(pending, expected: 100) == .pending)
assert(try! exitObservation(absent, expected: 100) == .absent)
for rejected in [changed, invalid, denied,
                 ProcessRead(metadata: .unavailable, bytes: 0, queryError: EACCES, killResult: -1, killError: ESRCH),
                 ProcessRead(metadata: .unavailable, bytes: 0, queryError: ESRCH, killResult: -1, killError: EPERM),
                 ProcessRead(metadata: .unavailable, bytes: 8, queryError: ESRCH, killResult: 0, killError: 0),
                 ProcessRead(metadata: .unavailable, bytes: 0, queryError: 0, killResult: 0, killError: 0)] {
    do { _ = try exitObservation(rejected, expected: 100); fatalError("Invalid observation accepted") } catch {}
}
for strictFailure in [pending, changed, invalid, denied] {
    do { _ = try strictKernelIsAlive(strictFailure, expected: 100); fatalError("Strict ownership relaxed") } catch {}
}
// Controlled clock and actions execute the actual stop sequence, not a model.
func sequence(_ owners: [ProcessRead], _ reads: [ProcessRead], forceRead: ProcessRead? = nil) -> (String, [Bool], Int, Double, Int) {
    var ownerIndex = 0, readIndex = 0
    var clock = 0.0
    var requests: [Bool] = []
    do {
        try stopOwnedProcess(owner: {
            let value = owners[min(ownerIndex, owners.count - 1)]; ownerIndex += 1
            return try strictKernelIsAlive(value, expected: 100)
        }, request: { requests.append($0) }, observe: {
            let value = requests.last == true ? (forceRead ?? reads.last!) : reads[min(readIndex, reads.count - 1)]
            readIndex += 1
            return try exitObservation(value, expected: 100)
        }, now: { clock }, pause: { clock += $0 }, phase: { _ in })
        return ("passed", requests, ownerIndex, clock, readIndex)
    } catch { return ("failed", requests, ownerIndex, clock, readIndex) }
}
let recovered = sequence([matching], [matching, pending, absent, changed])
assert(recovered.0 == "passed" && recovered.1 == [false] && recovered.2 == 1 && recovered.4 == 3) // No reread after absence.
let noOwner = sequence([pending], [absent])
assert(noOwner.0 == "failed" && noOwner.1.isEmpty && noOwner.4 == 0)
let wrongOwner = sequence([matching], [changed])
assert(wrongOwner.0 == "failed" && wrongOwner.1 == [false])
let unknownForce = sequence([matching, pending], [pending])
assert(unknownForce.0 == "failed" && unknownForce.1 == [false] && unknownForce.2 == 2 && unknownForce.3 == 10)
let changedForce = sequence([matching, changed], [matching])
assert(changedForce.0 == "failed" && changedForce.1 == [false] && changedForce.2 == 2)
let deadline = sequence([matching, matching], [matching], forceRead: pending)
assert(deadline.0 == "failed" && deadline.1 == [false, true] && deadline.2 == 2 && deadline.3 == 15)
let forceRecovered = sequence([matching, matching], [matching], forceRead: absent)
assert(forceRecovered.0 == "passed" && forceRecovered.1 == [false, true] && forceRecovered.2 == 2)
let absentBeforeForce = sequence([matching, absent], [matching])
assert(absentBeforeForce.0 == "passed" && absentBeforeForce.1 == [false] && absentBeforeForce.2 == 2)
let alreadyGone = sequence([absent], [changed])
assert(alreadyGone.0 == "passed" && alreadyGone.1.isEmpty && alreadyGone.4 == 0)
let permissionError = sequence([matching], [denied])
assert(permissionError.0 == "failed" && permissionError.1 == [false] && permissionError.3 == 0)

struct DiagnosticFailure: Error { let message: String }
func require(_ value: Bool, _ message: String) throws {
    if !value { throw DiagnosticFailure(message: message) }
}
func observe(_ pid: Int32, expected: UInt64, phase: String, sample: Int) -> [String: Any] {
    let read = readProcess(pid)
    let strict: String
    do { strict = try strictKernelIsAlive(read, expected: expected) ? "alive" : "absent" }
    catch { strict = "rejected" }
    let afterRequest: String
    do { afterRequest = String(describing: try exitObservation(read, expected: expected)) }
    catch { afterRequest = "rejected" }
    return ["pid": pid, "phase": phase, "sample": sample, "exactKernelDecision": strict,
            "postRequestDecision": afterRequest, "strictPreserved": afterRequest != "pending" || strict == "rejected", "metadata": String(describing: read.metadata),
            "queryBytes": read.bytes, "queryErrno": read.queryError,
            "killZeroReturn": read.killResult.map { $0 as Any } ?? NSNull(),
            "killZeroErrno": read.killError.map { $0 as Any } ?? NSNull()]
}
func experiment(_ mode: String) -> [String: Any] {
    var result: [String: Any] = ["mode": mode, "status": "failed", "reaped": false]
    var fds: [Int32] = [-1, -1]
    guard fds.withUnsafeMutableBufferPointer({ pipe($0.baseAddress!) }) == 0 else {
        result["error"] = "owned pipe creation failed"; return result
    }
    var actions: posix_spawn_file_actions_t?
    guard posix_spawn_file_actions_init(&actions) == 0 else {
        close(fds[0]); close(fds[1]); result["error"] = "spawn action init failed"; return result
    }
    let actionResults = [posix_spawn_file_actions_adddup2(&actions, fds[0], STDIN_FILENO),
                         posix_spawn_file_actions_addclose(&actions, fds[0]),
                         posix_spawn_file_actions_addclose(&actions, fds[1])]
    guard actionResults.allSatisfy({ $0 == 0 }) else {
        posix_spawn_file_actions_destroy(&actions)
        close(fds[0]); close(fds[1]); result["error"] = "spawn action setup failed"; return result
    }
    var arguments: [UnsafeMutablePointer<CChar>?] = [strdup("/bin/cat"), nil]
    var environment: [UnsafeMutablePointer<CChar>?] = [strdup("PATH=/usr/bin:/bin"), nil]
    var child: pid_t = 0
    let spawned = arguments.withUnsafeMutableBufferPointer { args in
        environment.withUnsafeMutableBufferPointer { values in
            posix_spawn(&child, "/bin/cat", &actions, nil, args.baseAddress!, values.baseAddress!)
        }
    }
    posix_spawn_file_actions_destroy(&actions)
    free(arguments[0]); free(environment[0])
    close(fds[0])
    guard spawned == 0, child > 0 else {
        close(fds[1]); result["error"] = "owned posix_spawn failed"; result["spawnReturn"] = spawned; return result
    }
    result["pid"] = child
    var writerOpen = true
    var reaped = false
    var observations: [[String: Any]] = []
    var waitObservations: [[String: Any]] = []
    var birth: UInt64 = 0
    var waitStatus: Int32 = 0
    do {
        for _ in 0..<50 {
            if let start = processStart(child) { birth = start; break }
            usleep(10_000)
        }
        try require(birth > 0, "could not establish child birth identity")
        result["receiptStart"] = birth
        for i in 0..<5 {
            let item = observe(child, expected: birth, phase: "held-alive", sample: i)
            observations.append(item)
            try require(item["exactKernelDecision"] as? String == "alive", "held child did not match its receipt")
            usleep(5_000)
        }
        if mode == "pipe-eof" {
            close(fds[1]); writerOpen = false
            result["exitRequest"] = "closed only this child's input pipe"
        } else {
            // Verify exact initial birth again before signaling our unreaped child.
            try require(processStart(child) == birth, "identity unavailable before owned SIGTERM")
            errno = 0
            let sent = kill(child, SIGTERM)
            result["signalReturn"] = sent
            result["signalErrno"] = errno
            try require(sent == 0, "owned SIGTERM failed")
            result["exitRequest"] = "SIGTERM to this exact owned child"
        }
        var exitedUnreaped = false
        for i in 0..<100 {
            observations.append(observe(child, expected: birth, phase: "exit-transition", sample: i))
            var information = siginfo_t()
            errno = 0
            let value = waitid(P_PID, id_t(child), &information, WEXITED | WNOHANG | WNOWAIT)
            let waitErrno = errno
            waitObservations.append(["sample": i, "return": value, "errno": waitErrno, "observedPid": information.si_pid])
            try require(value == 0, "waitid failed before reaping")
            if information.si_pid == child { exitedUnreaped = true; break }
            usleep(5_000)
        }
        try require(exitedUnreaped, "child did not reach exited/unreaped state within 500 ms")
        result["waitidConfirmedExitedWithoutReaping"] = true
        for i in 0..<3 {
            let observation = observe(child, expected: birth, phase: "exited-unreaped", sample: i)
            observations.append(observation)
            try require(observation["postRequestDecision"] as? String != "rejected", "owned exit observation was invalid")
            usleep(5_000)
        }
        errno = 0
        let waited = waitpid(child, &waitStatus, WNOHANG)
        result["waitpidReturn"] = waited
        result["waitpidErrno"] = errno
        try require(waited == child, "waitpid did not reap the confirmed exited child")
        reaped = true
        let after = observe(child, expected: birth, phase: "reaped", sample: 0)
        observations.append(after)
        try require(after["exactKernelDecision"] as? String == "absent", "reaped child absence was not confirmed")
        try require(observations.allSatisfy { $0["strictPreserved"] as? Bool == true }, "pending observation relaxed a strict caller")
        result["status"] = "passed"
    } catch {
        result["error"] = (error as? DiagnosticFailure)?.message ?? String(describing: error)
    }
    if writerOpen { close(fds[1]) }
    if !reaped {
        // This child has never been reaped, so its PID cannot have been reused.
        errno = 0
        result["cleanupKillReturn"] = kill(child, SIGKILL)
        result["cleanupKillErrno"] = errno
        for _ in 0..<200 {
            let waited = waitpid(child, &waitStatus, WNOHANG)
            if waited == child { reaped = true; break }
            if waited == -1 { break }
            usleep(5_000)
        }
    }
    result["reaped"] = reaped
    result["waitStatus"] = waitStatus
    result["observations"] = observations
    result["waitidObservations"] = waitObservations
    if !reaped { result["status"] = "failed"; result["cleanupError"] = "owned child reaping was not confirmed" }
    return result
}
let children = [experiment("pipe-eof"), experiment("sigterm")]
let passed = children.allSatisfy { $0["status"] as? String == "passed" && $0["reaped"] as? Bool == true }
let report: [String: Any] = ["status": passed ? "passed" : "failed", "localOS": ProcessInfo.processInfo.operatingSystemVersionString,
    "pureStopSequences": 10, "children": children, "xtraceLaunched": false]
let bytes = try JSONSerialization.data(withJSONObject: report, options: [.sortedKeys])
FileHandle.standardOutput.write(bytes); print("")
exit(passed ? 0 : 1)
