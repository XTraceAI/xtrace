func processStart(_ pid: Int32) -> UInt64? {
    let read = readProcess(pid)
    guard read.metadata != .invalid else { fail("Kernel process identity fields are invalid") }
    return read.start
}