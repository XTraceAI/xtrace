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
