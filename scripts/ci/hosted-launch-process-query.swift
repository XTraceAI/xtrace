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
