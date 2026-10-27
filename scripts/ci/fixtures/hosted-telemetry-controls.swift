let owned = URL(fileURLWithPath: "/owned/artifacts/hosted-telemetry/run-control/staging.noindex")
let envURL = owned.appendingPathComponent("environment-notice.json")
let probeReceipt = owned.appendingPathComponent("probe-receipt-notice.json")
let productReceipt = owned.deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent("launch-receipt-notice.json")
let probe = try! launchContext("probe-launch", environment: envURL, receipt: probeReceipt)
let product = try! launchContext("telemetry-launch", environment: envURL, receipt: productReceipt)
assert(probe.identifier == "ai.xtrace.app.test.telemetry-proxy")
assert(product.identifier == "ai.xtrace.app" && product.bundle.path == "/Applications/XTrace.app")
// Command roles stay distinct even though both releases use XTrace.app.
let anyReceipt = URL(fileURLWithPath: "/owned/artifacts/hosted-update/launch-receipt.json")
let anyEnvironment = URL(fileURLWithPath: "/owned/artifacts/hosted-update/run-x/staging.noindex/environment.json")
for (raw, path, command) in [("launch", "/Applications/XTrace.app", "launch"), ("quit", "/Applications/XTrace.app", "quit"),
                             ("absent", "/Applications/XTrace.app", "absent"),
                             ("earlier-launch", "/Applications/XTrace.app", "launch"),
                             ("earlier-quit", "/Applications/XTrace.app", "quit"),
                             ("earlier-arm-update", "/Applications/XTrace.app", "arm-update"),
                             ("earlier-adopt-update", "/Applications/XTrace.app", "adopt-update")] {
    let selected = try! launchContext(raw, environment: anyEnvironment, receipt: anyReceipt)
    assert(selected.command == command && selected.bundle.path == path && selected.identifier == "ai.xtrace.app" && !selected.telemetry)
    assert(selected.executable.path == path + "/Contents/MacOS/xtrace-desktop")
}
for raw in ["arm-update", "adopt-update", "telemetry-quit", "probe-quit", "earlier-telemetry-launch", "earlier-unknown", "earlier-", "later-launch"] {
    do { _ = try launchContext(raw, environment: anyEnvironment, receipt: anyReceipt); fatalError("Unsupported product command accepted: \(raw)") } catch {}
}
let earlierReceipt = Receipt(pid: 10, startTime: 100, bundle: ProductBundle.earlier, executable: ProductBundle.earlier + "/Contents/MacOS/xtrace-desktop", identifier: "ai.xtrace.app")
let earlierContext = try! launchContext("earlier-stop", environment: anyEnvironment, receipt: anyReceipt)
assert(receiptMatchesContext(earlierReceipt, earlierContext) && receiptMatchesContext(earlierReceipt, product))
for wrong in [
    Receipt(pid: 10, startTime: 100, bundle: "/Applications/Other.app", executable: earlierReceipt.executable, identifier: earlierReceipt.identifier),
    Receipt(pid: 10, startTime: 100, bundle: earlierReceipt.bundle, executable: "/wrong", identifier: earlierReceipt.identifier),
    Receipt(pid: 10, startTime: 100, bundle: earlierReceipt.bundle, executable: earlierReceipt.executable, identifier: "ai.xtrace.desktop")
] {
    assert(!receiptMatchesContext(wrong, earlierContext) && !receiptMatchesContext(wrong, product))
}
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

// Synthetic WebKit alert identities; captured hosted rows remain private.
var applicationAlert = alert
applicationAlert[kAXRoleAttribute] = "AXGroup"
applicationAlert[kAXSubroleAttribute] = "AXApplicationAlert"
applicationAlert["captureParent"] = "branch"
applicationAlert.removeValue(forKey: kAXHelpAttribute)
applicationAlert[kAXDescriptionAttribute] = blockedUpdaterText
let groupedAlert = [workspace, row("branch", "0", "AXGroup"), applicationAlert, good[2], retry]
assert(isAlert(alert) && isAlert(applicationAlert))
var missingSubrole = groupedAlert
missingSubrole[2].removeValue(forKey: kAXSubroleAttribute)
assert(!isAlert(missingSubrole[2]) && !telemetryAlerts(missingSubrole).safe)
var wrongSubrole = groupedAlert
wrongSubrole[2][kAXSubroleAttribute] = "AXOther"
assert(!isAlert(wrongSubrole[2]) && !telemetryAlerts(wrongSubrole).safe)
var wrongRole = groupedAlert
wrongRole[2][kAXRoleAttribute] = "AXButton"
assert(!isAlert(wrongRole[2]) && !telemetryAlerts(wrongRole).safe)
let supportedAlertCases: [[[String: String]]] = [good, groupedAlert]
for sample in supportedAlertCases {
    assert(telemetryAlerts(sample).safe && telemetryAlerts(sample).updaterWarning)
    assert(nativeError(sample)) // Ordinary first-launch and update modes reject both.
    var misplaced = sample; misplaced[0][kAXDescriptionAttribute] = "Other"
    assert(!telemetryAlerts(misplaced).safe)
    assert(!telemetryAlerts(sample.filter { !isButton($0, "Retry update") }).safe)
    var secondRetry = retry; secondRetry["captureNode"] = "retry-copy"
    assert(!telemetryAlerts(sample + [secondRetry]).safe)
    var different = sample
    different[different.firstIndex { $0["captureNode"] == "2" }!][kAXValueAttribute] = "Changed warning"
    assert(!telemetryAlerts(different).safe)
    assert(!telemetryAlerts(sample + [row("extra-text", "1", "AXStaticText", "Unrelated content")]).safe)
    assert(!telemetryAlerts(sample + [row("outside-copy", "0", "AXStaticText", blockedUpdaterText)]).safe)
    var secondAlert = sample.first(where: isAlert)!; secondAlert["captureNode"] = "alert-copy"
    assert(!telemetryAlerts(sample + [secondAlert]).safe)
    let unrelatedLegacy = row("unrelated", "0", "AXAlert", "Other warning")
    var unrelatedGroup = row("unrelated", "0", "AXGroup", "Other warning")
    unrelatedGroup[kAXSubroleAttribute] = "AXApplicationAlert"
    let unrelatedAlerts: [[String: String]] = [unrelatedLegacy, unrelatedGroup]
    for extra in unrelatedAlerts { assert(!telemetryAlerts(sample + [extra]).safe) }
    unrelatedGroup["captureParent"] = "1"
    assert(!telemetryAlerts(sample + [unrelatedGroup]).safe)
}

// Notice phase on 0.1.8: the Welcome notice is only found in the tree (the
// pinned bar keeps it from counting as shown). The phase must press Open
// Dashboard itself and then find the corner card; finding is never a pass.
func screen(_ texts: [String], _ buttons: [(String, Bool)]) -> [[String: String]] {
    [[kAXRoleAttribute: kAXWindowRole], [kAXRoleAttribute: "AXWebArea"]] + texts.map { [kAXValueAttribute: $0] } +
        buttons.map { [kAXRoleAttribute: kAXButtonRole, kAXTitleAttribute: $0.0, "captureEnabled": $0.1 ? "true" : "false"] }
}
let welcomeTexts = ["Welcome to XTrace", "The initial scan finished.", "Bundled readers verified", "In the local database now"] + noticeWords
let welcome = screen(welcomeTexts, [("Open Dashboard", true), ("Turn off", true)])
let cardTexts = ["Anonymous usage data is on by default", "App and macOS version", "A random ID made on this Mac", "Prompts or conversation text"]
let dashboardCard = screen(["What your agents did"] + cardTexts, [("Got it", true), ("Turn off", true)])
assert(telemetryDashboardNoticeStep(welcome, pressed: false) == .press("Open Dashboard"))
// Found without this run's press: never done, whatever the tree holds.
for rows in [welcome, dashboardCard, welcome + dashboardCard] { assert(telemetryDashboardNoticeStep(rows, pressed: false) != .done) }
assert(telemetryDashboardNoticeStep(dashboardCard, pressed: false) == .wait)
assert(telemetryDashboardNoticeStep(dashboardCard, pressed: true) == .done)
// After the press: still Welcome, no card, the Welcome notice only, a missing
// card fact, or a disabled/duplicate Turn off keeps waiting.
for rows in [welcome, screen(["What your agents did"], []), screen(["What your agents did"] + noticeWords, [("Turn off", true)]),
             screen(["What your agents did"] + cardTexts.dropLast(), [("Turn off", true)]),
             screen(["What your agents did"] + cardTexts, [("Turn off", false)]),
             screen(["What your agents did"] + cardTexts, [("Turn off", true), ("Turn off", true)]),
             dashboardCard + [[kAXValueAttribute: "Welcome to XTrace"]]] {
    assert(telemetryDashboardNoticeStep(rows, pressed: true) == .wait)
}
var loadingWelcome = welcome; loadingWelcome.append([kAXDescriptionAttribute: "Reading database counts"])
assert(telemetryDashboardNoticeStep(loadingWelcome, pressed: false) == .wait)
// Turn off happens in the card on the Dashboard, by this run's own press.
assert(telemetryTurnOffStep(dashboardCard, pressed: false) == .press("Turn off"))
assert(telemetryTurnOffStep(welcome, pressed: false) == .wait)
let cardOff = screen(["What your agents did", "Anonymous usage data is off", "App and macOS version", "Telemetry is off. Nothing is sent."], [("Got it", true)])
assert(telemetryTurnOffStep(cardOff, pressed: false) == .wait)
assert(telemetryTurnOffStep(cardOff, pressed: true) == .done)
for rows in [dashboardCard, cardOff + [[kAXRoleAttribute: kAXButtonRole, kAXTitleAttribute: "Turn off"]],
             screen(["What your agents did", "Telemetry is off. Nothing is sent. You can turn it back on in Settings."], []),
             cardOff + [[kAXValueAttribute: "Welcome to XTrace"]]] {
    assert(telemetryTurnOffStep(rows, pressed: true) == .wait)
}
assert(!telemetryCard(welcome) && telemetryCard(dashboardCard) && !telemetryCard(cardOff))
let reasonModes = ["telemetry-notice", "telemetry-dashboard-notice", "telemetry-turn-off", "telemetry-settings-off",
                   "telemetry-settings-on", "telemetry-settings-enable", "telemetry-settings-disable", "telemetry-environment-off"]
let reasons = Dictionary(uniqueKeysWithValues: reasonModes.map { ($0, telemetryReason($0)) })
assert(!reasons.values.contains { $0.contains("observed") || $0 == telemetryReason("unknown") })
print("TELEMETRY-REASONS " + String(decoding: try! JSONSerialization.data(withJSONObject: reasons, options: [.sortedKeys]), as: UTF8.self))
print("Telemetry notice-card step controls passed")
