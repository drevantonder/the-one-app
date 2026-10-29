// TheOneApp's menu bar: an MCP Apps host. It asks the node for the menu bar's
// items (the "menubar" tool), and shows each item's view, an HTML resource,
// in a sandboxed web view that talks to the node through this host.
import AppKit
import WebKit

let nodeURL = URL(string: "http://" + (ProcessInfo.processInfo.environment["ONE_ADDR"] ?? "127.0.0.1:4747"))!
let width: CGFloat = 300
let maxHeight: CGFloat = 600

enum MCPError: Error, LocalizedError {
    case rpc(String)
    case sessionExpired
    case badResponse

    var errorDescription: String? {
        switch self {
        case .rpc(let m): return m
        case .sessionExpired: return "the node forgot this session"
        case .badResponse: return "the node sent something unexpected"
        }
    }
}

// MCPClient speaks MCP's Streamable HTTP transport to the node.
actor MCPClient {
    private let endpoint = nodeURL.appendingPathComponent("mcp")
    private var session: String?
    private var connecting: Task<Void, Error>?
    private var nextID = 1

    func request(_ method: String, _ params: [String: Any] = [:]) async throws -> [String: Any] {
        try await connect()
        do {
            return try await send(method, params)
        } catch MCPError.sessionExpired {
            session = nil
            try await connect()
            return try await send(method, params)
        }
    }

    private func connect() async throws {
        if session != nil { return }
        if let connecting { return try await connecting.value }
        let task = Task {
            _ = try await self.send("initialize", [
                "protocolVersion": "2025-06-18", "capabilities": [:] as [String: Any],
                "clientInfo": ["name": "TheOneApp menu bar", "version": "1"],
            ])
            _ = try await self.post(["jsonrpc": "2.0", "method": "notifications/initialized"])
        }
        connecting = task
        defer { connecting = nil }
        try await task.value
    }

    private func send(_ method: String, _ params: [String: Any]) async throws -> [String: Any] {
        let id = nextID
        nextID += 1
        let (data, resp) = try await post(["jsonrpc": "2.0", "id": id, "method": method, "params": params])
        if let s = resp.value(forHTTPHeaderField: "Mcp-Session-Id") { session = s }
        if resp.statusCode == 404 && method != "initialize" { throw MCPError.sessionExpired }
        let messages: [[String: Any]]
        if resp.value(forHTTPHeaderField: "Content-Type")?.contains("text/event-stream") == true {
            messages = events(data)
        } else {
            messages = [(try? JSONSerialization.jsonObject(with: data)) as? [String: Any]].compactMap { $0 }
        }
        guard let reply = messages.first(where: { ($0["id"] as? Int) == id }) else { throw MCPError.badResponse }
        if let err = reply["error"] as? [String: Any] { throw MCPError.rpc(err["message"] as? String ?? "error") }
        return reply["result"] as? [String: Any] ?? [:]
    }

    private func post(_ body: [String: Any]) async throws -> (Data, HTTPURLResponse) {
        var req = URLRequest(url: endpoint, timeoutInterval: 60)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.setValue("application/json, text/event-stream", forHTTPHeaderField: "Accept")
        req.setValue("2025-06-18", forHTTPHeaderField: "MCP-Protocol-Version")
        if let session { req.setValue(session, forHTTPHeaderField: "Mcp-Session-Id") }
        req.httpBody = try JSONSerialization.data(withJSONObject: body)
        let (data, resp) = try await URLSession.shared.data(for: req)
        guard let http = resp as? HTTPURLResponse else { throw MCPError.badResponse }
        return (data, http)
    }

    private func events(_ data: Data) -> [[String: Any]] {
        let text = String(decoding: data, as: UTF8.self).replacingOccurrences(of: "\r\n", with: "\n")
        return text.components(separatedBy: "\n\n").compactMap { event in
            let payload = event.split(separator: "\n")
                .filter { $0.hasPrefix("data:") }
                .map { $0.dropFirst(5).drop(while: { $0 == " " }) }
                .joined(separator: "\n")
            return (try? JSONSerialization.jsonObject(with: Data(payload.utf8))) as? [String: Any]
        }
    }
}

// NodeProcess starts `one serve` from the app, unless a node already answers
// (one run by launchd, say), and stops it again on quit.
final class NodeProcess {
    private var process: Process?

    func healthy() async -> Bool {
        let req = URLRequest(url: nodeURL.appendingPathComponent("healthz"), timeoutInterval: 1)
        guard let resp = try? await URLSession.shared.data(for: req).1 else { return false }
        return (resp as? HTTPURLResponse)?.statusCode == 200
    }

    func ensureRunning() async {
        if await healthy() { return }
        let logs = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Logs/TheOneApp")
        try? FileManager.default.createDirectory(at: logs, withIntermediateDirectories: true)
        let log = logs.appendingPathComponent("node.log")
        if !FileManager.default.fileExists(atPath: log.path) { FileManager.default.createFile(atPath: log.path, contents: nil) }
        let out = try? FileHandle(forWritingTo: log)
        _ = try? out?.seekToEnd()
        let p = Process()
        p.executableURL = Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/one")
        p.arguments = ["serve"]
        p.standardOutput = out
        p.standardError = out
        do { try p.run() } catch { NSLog("starting the node: \(error)"); return }
        process = p
        for _ in 0..<50 {
            if await healthy() { return }
            try? await Task.sleep(nanoseconds: 200_000_000)
        }
    }

    func stop() {
        guard let p = process, p.isRunning else { return }
        p.terminate()
        p.waitUntilExit()
    }
}

func json(_ v: Any) -> String {
    guard let d = try? JSONSerialization.data(withJSONObject: v, options: [.fragmentsAllowed]) else { return "null" }
    return String(decoding: d, as: UTF8.self)
}

func theme() -> String {
    NSApp.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua ? "dark" : "light"
}

// The shell page holds the view in an iframe with an opaque origin, so the
// view reaches nothing but this host. The CSP denies the network unless the
// view's _meta.ui.csp lists domains.
func shell(csp: [String: Any]?) -> String {
    let connect = (csp?["connectDomains"] as? [String] ?? []).joined(separator: " ")
    let resource = (csp?["resourceDomains"] as? [String] ?? []).joined(separator: " ")
    let policy = "default-src 'none'; script-src 'unsafe-inline' \(resource); style-src 'unsafe-inline' \(resource); "
        + "img-src data: \(resource); font-src data: \(resource); media-src data: \(resource); connect-src \(connect.isEmpty ? "'none'" : connect)"
    return """
    <!doctype html><meta charset="utf-8">
    <meta http-equiv="Content-Security-Policy" content="\(policy)">
    <style>:root{color-scheme:\(theme())}html,body{margin:0;height:100%;background:transparent}iframe{display:block;border:0;width:100%;height:100%;background:transparent}</style>
    <iframe id="view" sandbox="allow-scripts"></iframe>
    <script>
    const view = document.getElementById('view');
    addEventListener('message', e => { if (e.source === view.contentWindow) webkit.messageHandlers.bridge.postMessage(JSON.stringify(e.data)); });
    function load(html) { view.srcdoc = html; }
    function deliver(message) { view.contentWindow.postMessage(message, '*'); }
    </script>
    """
}

// A Section is one menu bar item: its view, and the bridge between the view
// and the node.
@MainActor
final class Section: NSObject, WKScriptMessageHandler, WKNavigationDelegate {
    let title: String
    let tool: String
    let html: String
    let web: WKWebView
    let height: NSLayoutConstraint
    weak var panel: Panel?
    private var result: [String: Any]
    private var initialized = false

    init(item: [String: Any], html: String, csp: [String: Any]?, result: [String: Any], panel: Panel) {
        title = item["title"] as? String ?? ""
        tool = item["tool"] as? String ?? ""
        self.html = html
        self.result = result
        self.panel = panel
        let config = WKWebViewConfiguration()
        web = WKWebView(frame: NSRect(x: 0, y: 0, width: width, height: 1), configuration: config)
        web.setValue(false, forKey: "drawsBackground")
        web.translatesAutoresizingMaskIntoConstraints = false
        height = web.heightAnchor.constraint(equalToConstant: 1)
        height.isActive = true
        super.init()
        config.userContentController.add(self, name: "bridge")
        web.navigationDelegate = self
        web.loadHTMLString(shell(csp: csp), baseURL: nil)
    }

    func close() {
        web.configuration.userContentController.removeScriptMessageHandler(forName: "bridge")
        web.removeFromSuperview()
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        web.evaluateJavaScript("load(\(json(html)))")
    }

    func update(_ result: [String: Any]) {
        self.result = result
        if initialized { notify("ui/notifications/tool-result", result) }
    }

    func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
        guard let body = message.body as? String,
              let m = (try? JSONSerialization.jsonObject(with: Data(body.utf8))) as? [String: Any],
              let method = m["method"] as? String else { return }
        let id = m["id"]
        let params = m["params"] as? [String: Any] ?? [:]
        switch method {
        case "ui/initialize":
            reply(id, [
                "protocolVersion": "2026-01-26",
                "hostInfo": ["name": "TheOneApp", "version": "1"],
                "hostCapabilities": ["openLinks": [:] as [String: Any], "serverTools": [:] as [String: Any]],
                "hostContext": [
                    "theme": theme(), "platform": "desktop", "displayMode": "inline", "availableDisplayModes": ["inline"],
                    "containerDimensions": ["width": width, "maxHeight": maxHeight],
                ],
            ])
        case "ui/notifications/initialized":
            initialized = true
            notify("ui/notifications/tool-input", ["arguments": [:] as [String: Any]])
            notify("ui/notifications/tool-result", result)
        case "ui/notifications/size-changed":
            if let h = params["height"] as? Double {
                height.constant = min(CGFloat(h), maxHeight)
                panel?.fit()
            }
        case "ui/open-link":
            if let url = (params["url"] as? String).flatMap(URL.init(string:)) { NSWorkspace.shared.open(url) }
            reply(id, [:])
        case "tools/call":
            guard let panel else { return }
            Task {
                do {
                    reply(id, try await panel.client.request("tools/call", params))
                    await panel.refreshResults()
                } catch {
                    fail(id, -32603, error.localizedDescription)
                }
            }
        default:
            if id != nil { fail(id, -32601, "\(method) isn't supported by this host") }
        }
    }

    private func reply(_ id: Any?, _ result: [String: Any]) {
        guard let id else { return }
        deliver(["jsonrpc": "2.0", "id": id, "result": result])
    }

    private func fail(_ id: Any?, _ code: Int, _ message: String) {
        guard let id else { return }
        deliver(["jsonrpc": "2.0", "id": id, "error": ["code": code, "message": message]])
    }

    private func notify(_ method: String, _ params: [String: Any]) {
        deliver(["jsonrpc": "2.0", "method": method, "params": params])
    }

    private func deliver(_ message: [String: Any]) {
        web.evaluateJavaScript("deliver(\(json(message)))")
    }
}

@MainActor
final class Panel: NSViewController {
    let client = MCPClient()
    weak var popover: NSPopover?
    private let stack = NSStackView()
    private var sections: [Section] = []

    override func loadView() {
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 2
        stack.edgeInsets = NSEdgeInsets(top: 8, left: 6, bottom: 6, right: 6)
        stack.translatesAutoresizingMaskIntoConstraints = false
        let root = NSView(frame: NSRect(x: 0, y: 0, width: width, height: 100))
        root.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: root.topAnchor),
            stack.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            stack.widthAnchor.constraint(equalToConstant: width),
        ])
        view = root
        show([message("Loading…")])
    }

    // reload asks the node for the menu bar afresh, as on every open.
    func reload() async {
        let items: [[String: Any]]
        do {
            let doc = try await client.request("tools/call", ["name": "menubar", "arguments": [:] as [String: Any]])
            items = (doc["structuredContent"] as? [String: Any])?["items"] as? [[String: Any]] ?? []
        } catch {
            show([message("TheOneApp isn't running."), message(error.localizedDescription, small: true)])
            return
        }
        var fresh: [Section] = []
        for item in items {
            guard let tool = item["tool"] as? String, let uri = item["resource"] as? String else { continue }
            do {
                let res = try await client.request("resources/read", ["uri": uri])
                guard let content = (res["contents"] as? [[String: Any]])?.first, let html = content["text"] as? String else { continue }
                let csp = ((content["_meta"] as? [String: Any])?["ui"] as? [String: Any])?["csp"] as? [String: Any]
                let result = try await client.request("tools/call", ["name": tool, "arguments": [:] as [String: Any]])
                fresh.append(Section(item: item, html: html, csp: csp, result: result, panel: self))
            } catch {
                NSLog("menu bar item \(item["id"] ?? uri): \(error)")
            }
        }
        sections.forEach { $0.close() }
        sections = fresh
        var views: [NSView] = []
        for s in sections {
            if !s.title.isEmpty { views.append(header(s.title)) }
            views.append(s.web)
        }
        show(views)
    }

    // refreshResults re-runs every item's tool, after a view changed something.
    func refreshResults() async {
        for s in sections {
            if let result = try? await client.request("tools/call", ["name": s.tool, "arguments": [:] as [String: Any]]) {
                s.update(result)
            }
        }
    }

    func fit() {
        stack.layoutSubtreeIfNeeded()
        let size = NSSize(width: width, height: min(stack.fittingSize.height, maxHeight + 120))
        preferredContentSize = size
        popover?.contentSize = size
    }

    private func show(_ views: [NSView]) {
        stack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for v in views + [separator(), quitButton()] {
            stack.addArrangedSubview(v)
            if v is WKWebView || v is NSBox {
                v.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -12).isActive = true
            }
        }
        fit()
    }

    private func header(_ title: String) -> NSView {
        let label = NSTextField(labelWithString: title)
        label.font = .systemFont(ofSize: 11, weight: .semibold)
        label.textColor = .secondaryLabelColor
        return padded(label, top: 6)
    }

    private func message(_ text: String, small: Bool = false) -> NSView {
        let label = NSTextField(wrappingLabelWithString: text)
        label.font = .systemFont(ofSize: small ? 11 : 13)
        label.textColor = small ? .secondaryLabelColor : .labelColor
        label.preferredMaxLayoutWidth = width - 28
        return padded(label, top: 4)
    }

    private func padded(_ v: NSView, top: CGFloat) -> NSView {
        let box = NSView()
        v.translatesAutoresizingMaskIntoConstraints = false
        box.addSubview(v)
        NSLayoutConstraint.activate([
            v.leadingAnchor.constraint(equalTo: box.leadingAnchor, constant: 8),
            v.trailingAnchor.constraint(lessThanOrEqualTo: box.trailingAnchor, constant: -8),
            v.topAnchor.constraint(equalTo: box.topAnchor, constant: top),
            v.bottomAnchor.constraint(equalTo: box.bottomAnchor, constant: -2),
        ])
        return box
    }

    private func separator() -> NSView {
        let box = NSBox()
        box.boxType = .separator
        return box
    }

    private func quitButton() -> NSView {
        let b = NSButton(title: "Quit TheOneApp", target: NSApp, action: #selector(NSApplication.terminate(_:)))
        b.isBordered = false
        b.alignment = .left
        b.keyEquivalent = "q"
        b.keyEquivalentModifierMask = .command
        return padded(b, top: 2)
    }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSPopoverDelegate {
    private let node = NodeProcess()
    private let panel = Panel()
    private let popover = NSPopover()
    private var item: NSStatusItem?

    func applicationDidFinishLaunching(_ notification: Notification) {
        popover.behavior = .transient
        popover.contentViewController = panel
        popover.delegate = self
        panel.popover = popover
        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        item.button?.image = NSImage(systemSymbolName: "point.3.connected.trianglepath.dotted", accessibilityDescription: "TheOneApp")
        item.button?.target = self
        item.button?.action = #selector(toggle(_:))
        self.item = item
        Task { await node.ensureRunning() }
    }

    // Opening the app again, from Spotlight say, shows the menu.
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if !popover.isShown, let button = item?.button { toggle(button) }
        return false
    }

    func applicationWillTerminate(_ notification: Notification) {
        node.stop()
    }

    @objc func toggle(_ sender: NSStatusBarButton) {
        if popover.isShown {
            popover.performClose(sender)
            return
        }
        popover.show(relativeTo: sender.bounds, of: sender, preferredEdge: .minY)
        NSApp.activate(ignoringOtherApps: true)
        Task { await panel.reload() }
    }
}

MainActor.assumeIsolated {
    let delegate = AppDelegate()
    let app = NSApplication.shared
    app.delegate = delegate
    app.setActivationPolicy(.accessory)
    app.run()
}
