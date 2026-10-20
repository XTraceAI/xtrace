let owned = URL(fileURLWithPath: "/owned/artifacts/hosted-telemetry/run-control/staging.noindex")
let envURL = owned.appendingPathComponent("environment-notice.json")
let probeReceipt = owned.appendingPathComponent("probe-receipt-notice.json")
let productReceipt = owned.deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent("launch-receipt-notice.json")
let probe = try! launchContext("probe-launch", environment: envURL, receipt: probeReceipt)
let product = try! launchContext("launch", environment: envURL, receipt: productReceipt)
assert(probe.identifier == "ai.xtrace.app.test.telemetry-proxy")
assert(product.identifier == "ai.xtrace.app" && product.bundle.path == "/Applications/XTrace Desktop.app")
assert(probeMetadataMatches(["CFBundleIdentifier":probe.identifier,"CFBundleExecutable":"TelemetryProxyProbe"]))
for wrong in [["CFBundleIdentifier":"ai.xtrace.app","CFBundleExecutable":"TelemetryProxyProbe"], ["CFBundleIdentifier":probe.identifier,"CFBundleExecutable":"xtrace-desktop"]] { assert(!probeMetadataMatches(wrong)) }
let receipt = Receipt(pid: 10, startTime: 100, bundle: probe.bundle.path, executable: probe.executable.path, identifier: probe.identifier)
assert(receiptMatchesContext(receipt, probe) && !receiptMatchesContext(receipt, product))
for raw in ["probe-arm-update", "probe-adopt-update", "probe-unknown", "telemetry-adopt-update", "telemetry-launch", "telemetry-inspect", "telemetry-stop", "telemetry-absent"] {
    do { _ = try launchContext(raw, environment: envURL, receipt: probeReceipt); fatalError("Unsupported probe command accepted") } catch {}
}
for bad in [URL(fileURLWithPath:"/Applications/environment-notice.json"), owned.appendingPathComponent("../environment-notice.json"), owned.appendingPathComponent("arbitrary.json")] {
    do { _ = try launchContext("probe-launch", environment: bad, receipt: probeReceipt); fatalError("Escaped probe path accepted") } catch {}
}
do { _ = try launchContext("probe-launch", environment: envURL, receipt: productReceipt); fatalError("Cross-kind receipt path accepted") } catch {}
let expected: [String: String] = ["PATH":"/usr/bin:/bin:/usr/sbin:/sbin","LANG":"en_US.UTF-8","HOME":"/Users/runner","TMPDIR":owned.path+"/tmp","XTRACE_NATIVE_HOME":owned.path+"/home","CLAUDE_CONFIG_DIR":owned.path+"/claude","GH_CONFIG_DIR":owned.path+"/gh","SSH_AUTH_SOCK":"","HTTPS_PROXY":"http://127.0.0.1:12345","https_proxy":"http://127.0.0.1:12345","NO_PROXY":"","no_proxy":""]
assert(telemetryEnvironmentMatches(expected, expected: expected, owned: owned, home: "/Users/runner"))
for (key,value) in [("NO_PROXY","*"),("no_proxy","us.i.posthog.com"),("https_proxy","http://elsewhere"),("ALL_PROXY",""),("CURL_CA_BUNDLE","/other"),("XTRACE_DATA_DIR","/other"),("XTRACE_FIXTURE","F1"),("HOME","/other")] {
    var changed=expected; changed[key]=value
    assert(!telemetryEnvironmentMatches(changed, expected: expected, owned: owned, home: "/Users/runner"))
    assert(!telemetryEnvironmentMatches(changed, expected: changed, owned: owned, home: "/Users/runner"))
}
assert(telemetryHelperEnvironmentIsSafe(["PATH":expected["PATH"]!,"LANG":expected["LANG"]!],home:"/Users/runner"))
assert(!telemetryHelperEnvironmentIsSafe(["PATH":expected["PATH"]!,"LANG":expected["LANG"]!,"HTTPS_PROXY":expected["HTTPS_PROXY"]!],home:"/Users/runner"))
print("Telemetry context/environment controls passed")
