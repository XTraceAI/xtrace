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
