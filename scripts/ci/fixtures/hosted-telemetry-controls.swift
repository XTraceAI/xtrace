let owned = URL(fileURLWithPath: "/owned/artifacts/hosted-telemetry/run-control/staging.noindex")
let envURL = owned.appendingPathComponent("environment-notice.json")
let probeReceipt = owned.appendingPathComponent("probe-receipt-notice.json")
let productReceipt = owned.deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent("launch-receipt-notice.json")
let probe = try! launchContext("probe-launch", environment: envURL, receipt: probeReceipt)
let product = try! launchContext("telemetry-launch", environment: envURL, receipt: productReceipt)
assert(probe.identifier == "ai.xtrace.app.test.telemetry-proxy")
assert(product.identifier == "ai.xtrace.app" && product.bundle.path == "/Applications/XTrace Desktop.app")
assert(probeMetadataMatches(["CFBundleIdentifier":probe.identifier,"CFBundleExecutable":"TelemetryProxyProbe"]))
for wrong in [["CFBundleIdentifier":"ai.xtrace.app","CFBundleExecutable":"TelemetryProxyProbe"], ["CFBundleIdentifier":probe.identifier,"CFBundleExecutable":"xtrace-desktop"]] { assert(!probeMetadataMatches(wrong)) }
let receipt = Receipt(pid: 10, startTime: 100, bundle: probe.bundle.path, executable: probe.executable.path, identifier: probe.identifier)
assert(receiptMatchesContext(receipt, probe) && !receiptMatchesContext(receipt, product))
for raw in ["probe-arm-update", "probe-adopt-update", "probe-unknown", "telemetry-adopt-update"] {
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
func row(_ id: String, _ parent: String, _ role: String, _ text: String = "") -> [String:String] { ["captureNode":id,"captureParent":parent,kAXRoleAttribute:role,kAXValueAttribute:text] }
let workspace = ["captureNode":"0","captureParent":"",kAXRoleAttribute:"AXGroup",kAXSubroleAttribute:"AXLandmarkComplementary",kAXDescriptionAttribute:"Workspace"]
let alert = ["captureNode":"1","captureParent":"0","captureWorkspace":"true",kAXRoleAttribute:"AXAlert",kAXHelpAttribute:blockedUpdaterText]
let retry = ["captureNode":"3","captureParent":"1",kAXRoleAttribute:kAXButtonRole,kAXTitleAttribute:"Retry update"]
let good = [workspace,alert,row("2","1","AXStaticText",blockedUpdaterText),retry]
assert(telemetryAlerts(good).safe && telemetryAlerts(good).updaterWarning)
assert(nativeError(good)) // Ordinary modes still reject the same warning.
let rejectedAlertCases: [[[String: String]]] = [Array(good.dropLast()), [workspace,retry], [workspace,row("2","0","AXStaticText",blockedUpdaterText),retry],
            good+[row("4","0","AXAlert","other")], good+[row("4","0","AXDialog")],
            good+[row("4","0","AXStaticText","Database counts could not be read.")],
            good+[row("4","1","AXStaticText","other error")],
            good+[row("4","0","AXStaticText","Update installation failed. Retry installation.")]]
for bad in rejectedAlertCases {
    assert(!telemetryAlerts(bad).safe)
}
var outside = good; outside[0][kAXDescriptionAttribute] = "Other"; assert(!telemetryAlerts(outside).safe)
var changedText = good; changedText[2][kAXValueAttribute] = "Changed warning"; assert(!telemetryAlerts(changedText).safe)
var cycle = good; cycle[0]["captureParent"] = "1"; assert(!telemetryAlerts(cycle).safe)
var duplicate = good; duplicate.append(alert); assert(!telemetryAlerts(duplicate).safe)
let toggle = [kAXRoleAttribute:"AXCheckBox",kAXDescriptionAttribute:"Send anonymous usage data",kAXValueAttribute:"1","captureEnabled":"true"]
let settings = [[kAXValueAttribute:"Telemetry"],toggle]
assert(telemetrySwitchState(settings,on:true,enabled:true))
assert(!telemetrySwitchState(settings,on:false,enabled:true))
assert(!telemetrySwitchState(settings+[toggle],on:true,enabled:true))
print("Telemetry context/environment/alert controls passed")
let noticeWords = ["Anonymous usage data", "Telemetry is on by default. XTrace sends a few anonymous events, such as when the app is opened and how often it is used, so we can count how many people use it.", "The app version, the macOS version, which coding agents have history on this Mac (Claude, Codex, Cursor), and a random ID created on this Mac.", "Repository names, file paths, prompts or other conversation text, pull-request links, token counts or costs."]
let turnOff = [kAXRoleAttribute:kAXButtonRole,kAXTitleAttribute:"Turn off","captureEnabled":"true"]
let notice = noticeWords.map { [kAXValueAttribute:$0] } + [turnOff]
assert(telemetryNotice(notice))
for index in notice.indices { var missing=notice;missing.remove(at:index);assert(!telemetryNotice(missing)) }
var disabledNotice=notice;disabledNotice[4]["captureEnabled"]="false";assert(!telemetryNotice(disabledNotice))
var environmentOff=settings;environmentOff[1][kAXValueAttribute]="0";environmentOff[1]["captureEnabled"]="false"
assert(telemetrySwitchState(environmentOff,on:false,enabled:false))
assert(!telemetryAlerts(good+[row("4","0","AXStaticText","The update was installed, but restart failed. Retry restarting the app.")]).safe)
