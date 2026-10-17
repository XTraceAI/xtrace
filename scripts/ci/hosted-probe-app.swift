import AppKit
import WebKit

// The runner substitutes the UUID before compilation. This is a separate,
// harmless fixture, not a modified or re-signed XTrace product bundle.
let compiledIdentifier = "__PROBE_IDENTIFIER__"
guard compiledIdentifier.range(of: #"^ai\.xtrace\.app\.test\.[a-f0-9-]+$"#,
                               options: .regularExpression) != nil,
      Bundle.main.bundleIdentifier == compiledIdentifier else { exit(2) }

final class ProbeDelegate: NSObject, NSApplicationDelegate {
    var window: NSWindow!

    func applicationDidFinishLaunching(_ notification: Notification) {
        window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 520, height: 240),
                          styleMask: [.titled, .closable], backing: .buffered, defer: false)
        window.title = "Hosted AX capability probe"
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        let webView = WKWebView(frame: window.contentView!.bounds, configuration: configuration)
        webView.autoresizingMask = [.width, .height]
        window.contentView!.addSubview(webView)
        // Fixed offline fixture content only. The ordinary button handler runs
        // after the external AX press; the driver never injects JavaScript.
        webView.loadHTMLString("""
        <!doctype html><html lang="en"><head><meta charset="utf-8">
        <meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'">
        <title>Hosted AX capability probe</title></head><body>
        <p id="result">Awaiting AX action</p>
        <button type="button" onclick="document.getElementById('result').textContent='AX action confirmed'">Confirm AX action</button>
        </body></html>
        """, baseURL: nil)
        window.center()
        window.makeKeyAndOrderFront(nil)
        NSApplication.shared.activate(ignoringOtherApps: true)
    }

}

let app = NSApplication.shared
let delegate = ProbeDelegate()
app.setActivationPolicy(.regular)
app.delegate = delegate
app.run()
