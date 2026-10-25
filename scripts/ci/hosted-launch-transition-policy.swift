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
