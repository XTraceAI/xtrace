func waitForRefresh(until deadline: Date) {
    let sliceEnd = min(deadline, Date().addingTimeInterval(0.1))
    guard sliceEnd.timeIntervalSinceNow > 0 else { return }
    // The default mode belongs to the common set. NSWorkspace app-list changes
    // require a main-run-loop turn; .common is not itself a runnable mode.
    _ = RunLoop.main.run(mode: .default, before: sliceEnd)
    // An empty run loop can return immediately. Keep the polling slice bounded
    // without spinning or moving the caller's absolute deadline.
    if sliceEnd.timeIntervalSinceNow > 0 { Thread.sleep(until: sliceEnd) }
}
