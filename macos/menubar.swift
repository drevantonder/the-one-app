// The One App's menu bar: an MCP Apps host. It asks the node for the menu bar's
// items (the "menubar" tool), and shows each item's view, an HTML resource,
// in a sandboxed web view that talks to the node through this host.
import AppKit
import ServiceManagement
import SwiftUI
import WebKit

let nodeURL = URL(string: "http://" + (ProcessInfo.processInfo.environment["ONE_ADDR"] ?? "127.0.0.1:4747"))!
let width: CGFloat = 300
let maxHeight: CGFloat = 600

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
    let submenu: Bool
    let html: String
    let web: WKWebView
    let height: NSLayoutConstraint
    weak var panel: Panel?
    private var result: [String: Any]
    private var initialized = false
    private(set) var sized = false

    init(item: [String: Any], html: String, csp: [String: Any]?, result: [String: Any], panel: Panel) {
        title = item["title"] as? String ?? ""
        tool = item["tool"] as? String ?? ""
        submenu = item["submenu"] as? Bool ?? false
        self.html = html
        self.result = result
        self.panel = panel
        let config = WKWebViewConfiguration()
        web = WKWebView(frame: NSRect(x: 0, y: 0, width: width - 12, height: 1), configuration: config)
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
                "hostInfo": ["name": "The One App", "version": "1"],
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
                sized = true
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
    let client = MCPClient(node: nodeURL, name: "The One App menu bar")
    weak var popover: NSPopover?
    var openWindow: (() -> Void)?
    private let stack = NSStackView()
    private var sections: [Section] = []
    private var shown = ""  // the items and their views, as last shown
    private(set) var side: NSPanel?
    private var open: (section: Section, row: SubmenuRow)?

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
        root.addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseMoved, .activeAlways, .inVisibleRect], owner: self))
        show([message("Loading…")])
    }

    // Leaving the open submenu's row for another part of the menu closes it.
    override func mouseMoved(with event: NSEvent) {
        guard let row = open?.row else { return }
        if !row.bounds.contains(row.convert(event.locationInWindow, from: nil)) { closeSubmenu() }
    }

    func openSubmenu(_ section: Section, from row: SubmenuRow) {
        if open?.section === section { return }
        closeSubmenu()
        guard let window = view.window else { return }
        let side = self.side ?? SidePanel.make()
        self.side = side
        let content = side.contentView!
        section.web.removeFromSuperview()
        content.addSubview(section.web)
        NSLayoutConstraint.activate([
            section.web.topAnchor.constraint(equalTo: content.topAnchor, constant: 6),
            section.web.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 6),
            section.web.widthAnchor.constraint(equalToConstant: width - 12),
        ])
        open = (section, row)
        row.highlighted = true
        placeSubmenu()
        window.addChildWindow(side, ordered: .above)
        side.orderFront(nil)
    }

    func closeSubmenu() {
        guard let (section, row) = open else { return }
        section.web.removeFromSuperview()
        row.highlighted = false
        if let side {
            side.parent?.removeChildWindow(side)
            side.orderOut(nil)
        }
        open = nil
    }

    // placeSubmenu puts the submenu beside the menu, level with its row, or
    // on the menu's left if there's no room on the right.
    private func placeSubmenu() {
        guard let side, let (section, row) = open, let window = view.window else { return }
        let screen = window.screen?.visibleFrame ?? .infinite
        let rowTop = window.convertToScreen(row.convert(row.bounds, to: nil)).maxY
        let h = section.height.constant + 12
        let menu = window.convertToScreen(view.convert(view.bounds, to: nil))
        var x = menu.maxX + 4
        if x + width > screen.maxX { x = menu.minX - width - 4 }
        let top = min(rowTop + 6, screen.maxY)
        side.setFrame(NSRect(x: x, y: max(top - h, screen.minY), width: width, height: h), display: true)
    }

    // reload asks the node for the menu bar afresh, as on every open. If the
    // items and their views are the same as shown, it only refreshes their
    // data; otherwise it builds the new views out of sight and swaps them in
    // once they have their heights, so the popover doesn't shrink and regrow.
    func reload() async {
        let items: [[String: Any]]
        do {
            let doc = try await client.request("tools/call", ["name": "menubar", "arguments": [:] as [String: Any]])
            items = (doc["structuredContent"] as? [String: Any])?["items"] as? [[String: Any]] ?? []
        } catch {
            closeSubmenu()
            sections.forEach { $0.close() }
            sections = []
            shown = ""
            show([message("The One App isn't running."), message(error.localizedDescription, small: true)])
            return
        }
        var views: [(item: [String: Any], html: String, csp: [String: Any]?)] = []
        for item in items {
            guard let uri = item["resource"] as? String else { continue }
            do {
                let res = try await client.request("resources/read", ["uri": uri])
                guard let content = (res["contents"] as? [[String: Any]])?.first, let html = content["text"] as? String else { continue }
                let csp = ((content["_meta"] as? [String: Any])?["ui"] as? [String: Any])?["csp"] as? [String: Any]
                views.append((item, html, csp))
            } catch {
                NSLog("menu bar item \(item["id"] ?? uri): \(error)")
            }
        }
        let signature = json(views.map { [$0.item, $0.html] })
        if signature == shown && !sections.isEmpty {
            await refreshResults()
            return
        }
        var fresh: [Section] = []
        for v in views {
            guard let tool = v.item["tool"] as? String,
                  let result = try? await client.request("tools/call", ["name": tool, "arguments": [:] as [String: Any]]) else { continue }
            fresh.append(Section(item: v.item, html: v.html, csp: v.csp, result: result, panel: self))
        }
        for f in fresh {  // in the window but invisible, so they load and lay out
            f.web.alphaValue = 0
            f.web.frame.size.width = width - 12
            view.addSubview(f.web)
        }
        let deadline = Date().addingTimeInterval(1.5)
        while Date() < deadline && !fresh.allSatisfy(\.sized) {
            try? await Task.sleep(nanoseconds: 30_000_000)
        }
        closeSubmenu()
        sections.forEach { $0.close() }
        sections = fresh
        shown = signature
        fresh.forEach { $0.web.removeFromSuperview(); $0.web.alphaValue = 1 }
        var arranged: [NSView] = []
        for s in sections {
            if s.submenu {
                arranged.append(SubmenuRow(title: s.title) { [weak self] row in self?.openSubmenu(s, from: row) })
                continue
            }
            if !s.title.isEmpty { arranged.append(header(s.title)) }
            arranged.append(s.web)
        }
        show(arranged)
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
        placeSubmenu()
    }

    private func show(_ views: [NSView]) {
        stack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for v in views + [separator(), windowButton(), loginButton(), quitButton()] {
            stack.addArrangedSubview(v)
            if v is WKWebView || v is NSBox || v is SubmenuRow {
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

    // A borderless button draws its title greyed out while its window isn't
    // key, so the title gets an explicit color.
    private func menuButton(_ title: String, action: Selector, target: AnyObject) -> NSButton {
        let b = NSButton(title: title, target: target, action: action)
        b.isBordered = false
        b.alignment = .left
        b.attributedTitle = NSAttributedString(string: title, attributes: [.foregroundColor: NSColor.labelColor, .font: NSFont.systemFont(ofSize: 13)])
        return b
    }

    private func windowButton() -> NSView {
        padded(menuButton("Open The One App", action: #selector(openWindowClicked), target: self), top: 2)
    }

    @objc private func openWindowClicked() {
        popover?.performClose(nil)
        openWindow?()
    }

    private func loginButton() -> NSView {
        let b = menuButton("Open at Login", action: #selector(toggleLogin(_:)), target: self)
        b.setButtonType(.switch)
        b.state = SMAppService.mainApp.status == .enabled ? .on : .off
        return padded(b, top: 2)
    }

    @objc private func toggleLogin(_ sender: NSButton) {
        setOpenAtLogin(sender.state == .on)
        sender.state = SMAppService.mainApp.status == .enabled ? .on : .off
    }

    private func quitButton() -> NSView {
        let b = menuButton("Quit The One App", action: #selector(NSApplication.terminate(_:)), target: NSApp)
        b.keyEquivalent = "q"
        b.keyEquivalentModifierMask = .command
        return padded(b, top: 2)
    }
}

// A SubmenuRow is a menu row that opens its item's view beside the menu, on
// hover or click.
final class SubmenuRow: NSView {
    private let onOpen: (SubmenuRow) -> Void
    var highlighted = false { didSet { needsDisplay = true } }

    init(title: String, onOpen: @escaping (SubmenuRow) -> Void) {
        self.onOpen = onOpen
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        let label = NSTextField(labelWithString: title)
        label.font = .systemFont(ofSize: 13)
        let chevron = NSImageView(image: NSImage(systemSymbolName: "chevron.right", accessibilityDescription: nil) ?? NSImage())
        chevron.symbolConfiguration = .init(pointSize: 10, weight: .semibold)
        chevron.contentTintColor = .secondaryLabelColor
        for v in [label, chevron] as [NSView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            addSubview(v)
        }
        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: 24),
            label.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 8),
            label.centerYAnchor.constraint(equalTo: centerYAnchor),
            chevron.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
            chevron.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
        addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self))
    }

    required init?(coder: NSCoder) { nil }

    override func mouseEntered(with event: NSEvent) { onOpen(self) }
    override func mouseDown(with event: NSEvent) { onOpen(self) }

    override func draw(_ dirtyRect: NSRect) {
        guard highlighted else { return }
        NSColor.labelColor.withAlphaComponent(0.1).setFill()
        NSBezierPath(roundedRect: bounds, xRadius: 5, yRadius: 5).fill()
    }
}

// SidePanel holds an open submenu's view. It can become key, so the view's
// fields take typing.
final class SidePanel: NSPanel {
    override var canBecomeKey: Bool { true }

    static func make() -> SidePanel {
        let panel = SidePanel(contentRect: NSRect(x: 0, y: 0, width: width, height: 100), styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: true)
        panel.isFloatingPanel = true
        panel.level = .popUpMenu
        panel.backgroundColor = .clear
        panel.isOpaque = false
        panel.hasShadow = true
        let effect = NSVisualEffectView()
        effect.material = .popover
        effect.state = .active
        effect.wantsLayer = true
        effect.layer?.cornerRadius = 10
        effect.layer?.masksToBounds = true
        panel.contentView = effect
        return panel
    }
}

// The app runs the node, so it opens at login unless the user turns that off.
func setOpenAtLogin(_ on: Bool) {
    UserDefaults.standard.set(on, forKey: "openAtLogin")
    do {
        if on { try SMAppService.mainApp.register() } else { try SMAppService.mainApp.unregister() }
    } catch {
        NSLog("open at login: \(error)")
    }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSPopoverDelegate {
    private let node = NodeProcess()
    private let panel = Panel()
    private let popover = NSPopover()
    private var item: NSStatusItem?
    private var monitors: [Any] = []
    private var observers: [NSObjectProtocol] = []
    private var window: NSWindow?

    func applicationDidFinishLaunching(_ notification: Notification) {
        // Closed by hand, not .transient, so clicks in an open submenu don't close the menu.
        popover.behavior = .applicationDefined
        popover.animates = false  // open and close at once, like the system's menu bar menus
        popover.contentViewController = panel
        popover.delegate = self
        panel.popover = popover
        panel.openWindow = { [weak self] in self?.showWindow() }
        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        let ring = NSImage(named: "MenuBarIcon")
        ring?.isTemplate = true
        ring?.accessibilityDescription = "The One App"
        item.button?.image = ring
        item.button?.target = self
        item.button?.action = #selector(toggle(_:))
        self.item = item
        if UserDefaults.standard.object(forKey: "openAtLogin") == nil { setOpenAtLogin(true) }
        Task { await node.ensureRunning() }
        let built = executableID()
        Timer.scheduledTimer(withTimeInterval: 15, repeats: true) { _ in
            MainActor.assumeIsolated {
                if let now = self.executableID(), now != built { self.relaunch() }
            }
        }
    }

    // showWindow opens the app's window, the same shell the iPhone and iPad
    // app shows, on this Mac's node.
    func showWindow() {
        if window == nil {
            let host = NSHostingController(rootView: ShellView(shell: Shell(node: nodeURL)).frame(minWidth: 420, minHeight: 480))
            let w = NSWindow(contentViewController: host)
            w.title = "The One App"
            w.setContentSize(NSSize(width: 480, height: 600))
            w.isReleasedWhenClosed = false
            w.center()
            window = w
        }
        NSApp.activate()
        window?.makeKeyAndOrderFront(nil)
    }

    // `one update` replaces the bundle and restarts only the node, so the app
    // watches its own executable and relaunches itself, and the node, when
    // a new build lands.
    private func executableID() -> Int? {
        guard let path = Bundle.main.executablePath else { return nil }
        return (try? FileManager.default.attributesOfItem(atPath: path))?[.systemFileNumber] as? Int
    }

    private func relaunch() {
        node.stop()
        let open = Process()
        open.executableURL = URL(fileURLWithPath: "/usr/bin/open")
        open.arguments = ["-n", Bundle.main.bundlePath]
        try? open.run()
        NSApp.terminate(nil)
    }

    // Opening the app again, from Spotlight say, shows the menu.
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if !popover.isShown, let button = item?.button { toggle(button) }
        return false
    }

    // watchForClose closes the menu on a click outside it and its submenu, on
    // Escape, and when another app takes over.
    private func watchForClose() {
        guard monitors.isEmpty else { return }
        let ours = { [weak self] (w: NSWindow?) -> Bool in
            guard let self, let w else { return false }
            return w === self.panel.view.window || w === self.panel.side || w === self.item?.button?.window
        }
        if let m = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown, .keyDown], handler: { [weak self] e in
            if e.type == .keyDown {
                if e.keyCode == 53 { self?.close(); return nil }
                return e
            }
            if !ours(e.window) { self?.close() }
            return e
        }) { monitors.append(m) }
        if let m = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown], handler: { [weak self] _ in
            MainActor.assumeIsolated { self?.close() }
        }) { monitors.append(m) }
        observers.append(NotificationCenter.default.addObserver(forName: NSApplication.didResignActiveNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.close() }
        })
    }

    private func close() {
        if popover.isShown { popover.performClose(nil) }
    }

    func popoverDidClose(_ notification: Notification) {
        panel.closeSubmenu()
        monitors.forEach(NSEvent.removeMonitor)
        observers.forEach(NotificationCenter.default.removeObserver)
        monitors = []
        observers = []
    }

    func applicationWillTerminate(_ notification: Notification) {
        node.stop()
    }

    @objc func toggle(_ sender: NSStatusBarButton) {
        if popover.isShown {
            popover.performClose(sender)
            return
        }
        // The popover's window must be key for the views to take keyboard input.
        NSApp.activate()
        popover.show(relativeTo: sender.bounds, of: sender, preferredEdge: .minY)
        panel.view.window?.makeKey()
        watchForClose()
        Task { await panel.reload() }
    }
}

@main
enum Main {
    static func main() {
        MainActor.assumeIsolated {
            let delegate = AppDelegate()
            let app = NSApplication.shared
            app.delegate = delegate
            app.setActivationPolicy(.accessory)
            app.run()
        }
    }
}
