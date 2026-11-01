import ImageIO
import SwiftUI
import UniformTypeIdentifiers
import WebKit

/// An app's screen: the page the node serves for it, in a web view that answers its bridge.
struct AppScreen: View {
    @Environment(Store.self) private var store
    var route: ScreenRoute
    @State private var title: String?
    @State private var loaded = false
    @State private var slow = false
    @State private var failure: String?
    /// When the node couldn't be reached, so the screen shows its last copy, kept from then.
    @State private var offline: Date?

    var body: some View {
        ScreenWebView(route: route, store: store, title: $title, loaded: $loaded, failure: $failure, offline: $offline)
            .opacity(loaded ? 1 : 0)
            .safeAreaInset(edge: .top, spacing: 0) {
                if let offline {
                    Label("Offline: this is how it looked \(offline.formatted(.relative(presentation: .named))).", systemImage: "wifi.slash")
                        .font(.footnote.weight(.medium))
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 14)
                        .padding(.vertical, 7)
                        .glassEffect(.regular, in: .capsule)
                        .padding(.vertical, 6)
                }
            }
            .overlay {
                if let failure {
                    ContentUnavailableView("Can’t Open This", systemImage: "exclamationmark.triangle", description: Text(failure))
                } else if !loaded && slow {
                    ProgressView()
                }
            }
            .task {
                try? await Task.sleep(for: .milliseconds(400))
                slow = true
            }
            #if os(macOS)
            // Content fades out under the toolbar, as a native scroll view's does.
            .mask {
                VStack(spacing: 0) {
                    LinearGradient(colors: [.clear, .black], startPoint: .top, endPoint: .bottom).frame(height: 12)
                    Color.black
                }
            }
            #endif
            .wholeTitle(title ?? defaultTitle)
    }

    private var defaultTitle: String {
        let app = store.app(route.app)
        return app?.screens?.first { $0.name == route.screen }?.title ?? app?.label ?? ""
    }
}

#if os(macOS)
private typealias Representable = NSViewRepresentable

#else
private typealias Representable = UIViewRepresentable
#endif

private struct ScreenWebView: Representable {
    var route: ScreenRoute
    var store: Store
    @Binding var title: String?
    @Binding var loaded: Bool
    @Binding var failure: String?
    @Binding var offline: Date?
    @Environment(\.dynamicTypeSize) private var typeSize

    func makeCoordinator() -> Coordinator { Coordinator(view: self) }

    #if os(macOS)
    func makeNSView(context: Context) -> WKWebView { make(context) }
    func updateNSView(_ web: WKWebView, context: Context) { context.coordinator.view = self }
    #else
    func makeUIView(context: Context) -> WKWebView { make(context) }
    func updateUIView(_ web: WKWebView, context: Context) {
        context.coordinator.view = self
        web.evaluateJavaScript(zoomScript)
    }
    #endif

    private func make(_ context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.userContentController.addScriptMessageHandler(context.coordinator, contentWorld: .page, name: "one")
        config.userContentController.addUserScript(WKUserScript(source: look, injectionTime: .atDocumentStart, forMainFrameOnly: true))
        // What this shell can do beyond the bridge's basics, for screens to offer it only where it works.
        config.userContentController.addUserScript(WKUserScript(source: "window.__oneShell = { scan: \(ShellFiles.canScan) };", injectionTime: .atDocumentStart, forMainFrameOnly: true))
        config.userContentController.addUserScript(WKUserScript(source: Self.noZoom, injectionTime: .atDocumentEnd, forMainFrameOnly: true))
        let web = WKWebView(frame: .zero, configuration: config)
        web.navigationDelegate = context.coordinator
        web.uiDelegate = context.coordinator
        web.allowsLinkPreview = false
        #if os(macOS)
        web.setValue(false, forKey: "drawsBackground")
        #else
        web.isOpaque = false
        web.backgroundColor = .clear
        web.scrollView.backgroundColor = .clear
        web.scrollView.keyboardDismissMode = .interactive
        #endif
        context.coordinator.load(into: web)
        return web
    }

    /// The kit is sized for a phone's 17-point text: an iPhone's follows Dynamic Type (a Mac's is smaller: see platformCSS).
    /// It's CSS zoom, which lays the page out to the zoomed width; the web view's pageZoom only scales the drawing, so
    /// a page at any other text size was narrower than the screen or ran off it. It never leaves the page less than
    /// 320 CSS pixels wide, the narrowest phone layouts are made for, so the largest sizes zoom only as far as that.
    /// Before the page's viewport tag is read its width isn't the screen's, and reading the tag fires no resize, so it
    /// fits again once the page has loaded.
    #if os(macOS)
    private let zoomScript = ""
    #else
    private var zoomScript: String {
        let zoom = UIFontMetrics(forTextStyle: .body).scaledValue(for: 17, compatibleWith: UITraitCollection(preferredContentSizeCategory: UIContentSizeCategory(typeSize))) / 17
        return """
        window.__oneZoom = \(zoom);
        if (!window.__oneFit) {
          window.__oneFit = () => { const z = window.__oneZoom; document.documentElement.style.zoom = String(innerWidth ? Math.min(z, innerWidth / 320) : z); };
          addEventListener("resize", window.__oneFit);
          document.addEventListener("DOMContentLoaded", window.__oneFit);
          addEventListener("load", window.__oneFit);
        }
        window.__oneFit();
        """
    }
    #endif

    /// What makes a page sit in the shell as its own: the shell's background shows through, its accent is the
    /// shell's in light and dark, and nothing zooms.
    private var look: String {
        let accent = store.config?.accent ?? Bundle.main.object(forInfoDictionaryKey: "Accent") as? String ?? ""
        return """
        const style = document.createElement("style");
        style.textContent = `
          html, body { background: transparent !important; }
          html { -webkit-tap-highlight-color: transparent; -webkit-touch-callout: none; }
          :root:root { --accent: \(accentCSS(accent, dark: false)); }
          @media (prefers-color-scheme: dark) { :root:root { --accent: \(accentCSS(accent, dark: true)); } }
        \(Self.platformCSS)`;
        document.documentElement.appendChild(style);
        \(zoomScript)
        """
    }

    private static let noZoom = """
    document.querySelector('meta[name="viewport"]')?.setAttribute("content", "width=device-width,initial-scale=1,maximum-scale=1,viewport-fit=cover");
    """

    #if os(macOS)
    private static let platformCSS = """
      html { zoom: calc(13 / 17); }
      :root:root { --bg: transparent; --card: rgba(0, 0, 0, 0.04); }
      @media (prefers-color-scheme: dark) { :root:root { --card: rgba(255, 255, 255, 0.05); } }
    """
    #else
    private static let platformCSS = ""
    #endif

    @MainActor
    final class Coordinator: NSObject, WKScriptMessageHandlerWithReply, WKNavigationDelegate, WKUIDelegate, WKDownloadDelegate {
        var view: ScreenWebView
        private weak var web: WKWebView?

        init(view: ScreenWebView) {
            self.view = view
            super.init()
            NotificationCenter.default.addObserver(self, selector: #selector(nodeEvent), name: .nodeEvent, object: nil)
        }

        /// Node events reach the page as `one.__event(event)`; the store posts them on the main thread.
        @objc private func nodeEvent(_ note: Notification) {
            guard let json = note.object as? String else { return }
            web?.evaluateJavaScript("window.one && one.__event(\(json))")
        }

        func load(into web: WKWebView) {
            self.web = web
            let route = view.route, node = view.store.node
            Task {
                do {
                    let page = try await node.page(app: route.app, screen: route.screen, params: route.params, scope: route.scope)
                    web.loadHTMLString(page.html, baseURL: page.base)
                    ScreenCache.keep(page.html, for: route)
                    view.offline = nil
                } catch {
                    // Offline, the last copy of this screen stands in, saying so; its calls fail until the node answers.
                    if let kept = ScreenCache.last(for: route) {
                        web.loadHTMLString(kept.html, baseURL: nil)
                        view.offline = kept.time
                    } else {
                        view.failure = error.localizedDescription
                    }
                }
            }
        }

        #if os(iOS)
        private var host: UIView? { web }
        #else
        private var host: NSView? { web }
        #endif

        /// The bridge: `{ type: "call" | "open" | "title" | "icon" | "copy" | "haptic" | "save" | "preview" | "scan" | "vault", … }`.
        /// A page only ever calls its own app, but may open another's screens.
        func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) async -> (Any?, String?) {
            guard let body = message.body as? [String: Any] else { return (nil, "Not a message") }
            let route = view.route
            switch body["type"] as? String {
            case "call":
                do {
                    let input = try JSONSerialization.data(withJSONObject: body["input"] ?? [:], options: .fragmentsAllowed)
                    let result = try await view.store.node.call(app: route.app, op: body["op"] as? String ?? "", input: input)
                    return (try JSONSerialization.jsonObject(with: result, options: .fragmentsAllowed), nil)
                } catch {
                    return (nil, error.localizedDescription)
                }
            case "open":
                let params = (try? JSONSerialization.data(withJSONObject: body["params"] ?? [:])).map { String(decoding: $0, as: UTF8.self) } ?? "{}"
                let app = (body["app"] as? String).flatMap { $0.isEmpty ? nil : $0 } ?? route.app
                view.store.path.append(.screen(ScreenRoute(app: app, screen: body["screen"] as? String ?? "main", params: params, scope: route.scope)))
                return (nil, nil)
            case "save", "preview":
                do {
                    let file = try await ScreenFile.from(body["file"], node: view.store.node)
                    #if os(iOS)
                    if body["type"] as? String == "save" { try ShellFiles.save(file, from: host) } else { try ShellFiles.preview(file, from: host) }
                    #else
                    if body["type"] as? String == "save" { try await ShellFiles.save(file, from: host) } else { try ShellFiles.preview(file, from: host) }
                    #endif
                    return (nil, nil)
                } catch {
                    return (nil, error.localizedDescription)
                }
            case "scan":
                do {
                    return (try await ShellFiles.scan(from: host).map(\.json), nil)
                } catch {
                    return (nil, error.localizedDescription)
                }
            case "vault":
                // Asks for what the app declares, in the Vault's sheet, and answers once the sheet is gone, or once the
                // request is answered somewhere else (another device, an agent's CLI), which closes the sheet.
                let store = view.store, scope = route.scope ?? "user"
                do {
                    guard let id = try await store.node.requestItem(scope: scope, app: route.app, need: body["need"] as? String ?? "") else {
                        return (nil, "The Vault didn’t take the request")
                    }
                    let sheet = Sheet.vault(scope: scope, id: id)
                    store.sheet = sheet
                    var waited = 0
                    while store.sheet == sheet {
                        try? await Task.sleep(for: .milliseconds(300))
                        waited += 1
                        if waited % 7 == 0, let state = try? await store.node.vaultRequests(scope: scope, id: id).first?.state, state != "waiting", store.sheet == sheet {
                            store.sheet = nil
                        }
                    }
                    let state = try await store.node.vaultRequests(scope: scope, id: id).first?.state
                    return (state == "set", nil)
                } catch {
                    return (nil, error.localizedDescription)
                }
            case "title":
                view.title = body["text"] as? String
                return (nil, nil)
            case "icon":
                let tile = AppTile(icon: body["symbol"] as? String, seed: body["seed"] as? String ?? "", color: (body["color"] as? String).flatMap { $0.isEmpty ? nil : $0 },
                                   image: (body["image"] as? String).flatMap { $0.isEmpty ? nil : $0 },
                                   imageKind: body["kind"] as? String, size: body["size"] as? Double ?? 36)
                return (Self.png(tile), nil)
            case "copy":
                let text = body["text"] as? String ?? ""
                #if os(macOS)
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(text, forType: .string)
                #else
                UIPasteboard.general.string = text
                #endif
                return (nil, nil)
            case "haptic":
                #if os(iOS)
                switch body["style"] as? String {
                case "medium": UIImpactFeedbackGenerator(style: .medium).impactOccurred()
                case "success": UINotificationFeedbackGenerator().notificationOccurred(.success)
                case "error": UINotificationFeedbackGenerator().notificationOccurred(.error)
                default: UISelectionFeedbackGenerator().selectionChanged()
                }
                #endif
                return (nil, nil)
            default:
                return (nil, "Unknown message")
            }
        }

        /// A view as a PNG data URL, for an `<img>` or a CSS background.
        private static func png(_ content: some View) -> String? {
            let renderer = ImageRenderer(content: content)
            renderer.scale = 3
            let data = NSMutableData()
            guard let image = renderer.cgImage, let out = CGImageDestinationCreateWithData(data, "public.png" as CFString, 1, nil) else { return nil }
            CGImageDestinationAddImage(out, image, nil)
            guard CGImageDestinationFinalize(out) else { return nil }
            return "data:image/png;base64," + (data as Data).base64EncodedString()
        }

        func webView(_ web: WKWebView, decidePolicyFor action: WKNavigationAction) async -> WKNavigationActionPolicy {
            // `<a download>`, and a blob or data URL, save the file; once the screen has loaded, anything else that
            // would take it away (a link, `location.href`) leaves for the browser, Mail or Phone, and the screen stays.
            if action.shouldPerformDownload { return .download }
            guard shown, action.targetFrame?.isMainFrame != false, let url = action.request.url, url != web.url else { return .allow }
            switch url.scheme ?? "" {
            case "about", "javascript": return .allow
            case "blob", "data": return .download
            default:
                Self.openOutside(url)
                return .cancel
            }
        }

        /// Whether the screen has loaded, after which it never navigates away.
        private var shown = false

        func webView(_ web: WKWebView, decidePolicyFor response: WKNavigationResponse) async -> WKNavigationResponsePolicy {
            response.canShowMIMEType ? .allow : .download
        }

        /// `window.open` and `target="_blank"`: a web address in the browser; nothing opens inside the screen.
        func webView(_ web: WKWebView, createWebViewWith configuration: WKWebViewConfiguration, for action: WKNavigationAction,
                     windowFeatures: WKWindowFeatures) -> WKWebView? {
            if let url = action.request.url {
                if ["blob", "data"].contains(url.scheme ?? "") { web.load(action.request) } else { Self.openOutside(url) }
            }
            return nil
        }

        private static func openOutside(_ url: URL) {
            #if os(macOS)
            NSWorkspace.shared.open(url)
            #else
            UIApplication.shared.open(url)
            #endif
        }

        #if os(macOS)
        /// A file input, on a Mac: the open panel (an iPhone's web view has its own picker).
        func webView(_ web: WKWebView, runOpenPanelWith parameters: WKOpenPanelParameters, initiatedByFrame frame: WKFrameInfo) async -> [URL]? {
            let panel = NSOpenPanel()
            panel.allowsMultipleSelection = parameters.allowsMultipleSelection
            panel.canChooseDirectories = parameters.allowsDirectories
            panel.canChooseFiles = true
            let answer = if let window = web.window { await panel.beginSheetModal(for: window) } else { panel.runModal() }
            return answer == .OK ? panel.urls : nil
        }
        #endif

        // MARK: Downloads: a link to a file, a blob or data URL, saved like `one.save`.

        func webView(_ web: WKWebView, navigationAction: WKNavigationAction, didBecome download: WKDownload) { download.delegate = self }
        func webView(_ web: WKWebView, navigationResponse: WKNavigationResponse, didBecome download: WKDownload) { download.delegate = self }

        private var downloads: [ObjectIdentifier: URL] = [:]

        func download(_ download: WKDownload, decideDestinationUsing response: URLResponse, suggestedFilename: String) async -> URL? {
            let dir = FileManager.default.temporaryDirectory.appending(path: "screen-files/\(UUID().uuidString)")
            try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            let url = dir.appending(path: suggestedFilename.isEmpty ? "Download" : suggestedFilename)
            downloads[ObjectIdentifier(download)] = url
            return url
        }

        func downloadDidFinish(_ download: WKDownload) {
            guard let url = downloads.removeValue(forKey: ObjectIdentifier(download)), let data = try? Data(contentsOf: url) else { return }
            let file = ScreenFile(name: url.lastPathComponent, type: UTType(filenameExtension: url.pathExtension)?.preferredMIMEType ?? "application/octet-stream", data: data)
            Task {
                #if os(iOS)
                try? ShellFiles.save(file, from: host)
                #else
                try? await ShellFiles.save(file, from: host)
                #endif
            }
        }

        func download(_ download: WKDownload, didFailWithError error: Error, resumeData: Data?) {
            downloads[ObjectIdentifier(download)] = nil
        }

        func webView(_ web: WKWebView, didFinish navigation: WKNavigation!) {
            shown = true
            view.loaded = true
        }

        func webViewWebContentProcessDidTerminate(_ web: WKWebView) {
            shown = false
            load(into: web)
        }
    }
}

/// What a shell shows for a route: a page, an app's screen, or one of its own views.
struct RouteView: View {
    @Environment(Store.self) private var store
    var route: Route
    var open: (Route) -> Void

    var body: some View {
        switch route {
        case .page(let scope):
            if let page = store.page(scope) { PageView(page: page, open: open) }
        case .screen(let screen):
            AppScreen(route: screen).id(screen)
        case .settings:
            SettingsPage(open: open)
        case .group(let id):
            GroupView(id: id)
        case .services(let scope):
            if let page = store.page(scope) { ServicesView(page: page, open: open) }
        case .setting(let setting):
            switch setting {
            case .store:
                // The install's store app, once it exists; until then, a list of every app.
                if let app = store.app(for: .store) {
                    AppScreen(route: ScreenRoute(app: app.name)).id(app.name)
                } else {
                    AppStoreView()
                }
            case .groups: GroupsView(open: open)
            case .devices: DevicesView()
            case .agents: AgentsView()
            case .vault: VaultView()
            case .access: AccessLogView()
            case .users: UsersView()
            case .install: InstallSettingsView()
            case .backups: BackupsView()
            case .requests: RequestsView()
            case .appearance: AppearanceView()
            }
        }
    }
}

/// The last copy of each screen this device drew, by route, for when the node can't be reached.
enum ScreenCache {
    private static var dir: URL {
        let dir = URL.cachesDirectory.appending(path: "screens")
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private static func file(_ route: ScreenRoute) -> URL {
        let key = [route.app, route.screen, route.params, route.scope ?? ""].joined(separator: "\u{1}")
        return dir.appending(path: "\(steadyHash(key))-\(key.count).html")
    }

    static func keep(_ html: String, for route: ScreenRoute) {
        try? Data(html.utf8).write(to: file(route), options: .atomic)
    }

    static func last(for route: ScreenRoute) -> (html: String, time: Date)? {
        let url = file(route)
        guard let data = try? Data(contentsOf: url) else { return nil }
        let time = (try? url.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? .now
        return (String(decoding: data, as: UTF8.self), time)
    }
}
