import ImageIO
import SwiftUI
import UniformTypeIdentifiers
import WebKit

/// An app's screen, or a Settings screen (`settings`, its name as `route`'s screen and no app): the page the node
/// serves for it, in a web view that answers its bridge, a Settings screen's alone as the person.
struct AppScreen: View {
    @Environment(Store.self) private var store
    var route: ScreenRoute
    var settings = false
    /// For a screen in a sheet: closes it, with what the screen gave `one.close`.
    var dismiss: ((Any?) -> Void)?
    @State private var title: String?
    @State private var loaded = false
    @State private var slow = false
    @State private var failure: String?
    /// When the node couldn't be reached, so the screen shows its last copy, until it answers again.
    @State private var offline: Offline?
    /// Bumped by Try Now, for the web view to try the node again at once.
    @State private var tries = 0

    var body: some View {
        ScreenWebView(route: route, settings: settings, dismiss: dismiss, store: store, title: $title, loaded: $loaded, failure: $failure, offline: $offline, tries: tries)
            .opacity(loaded ? 1 : 0)
            .safeAreaInset(edge: .top, spacing: 0) {
                if let offline {
                    TimelineView(.periodic(from: .now, by: 30)) { context in
                        HStack(spacing: 10) {
                            Label(context.date.timeIntervalSince(offline.kept) < 60 ? "Offline" : "Offline · from \(offline.kept.formatted(.relative(presentation: .named, unitsStyle: .wide)))", systemImage: "wifi.slash")
                                .foregroundStyle(.secondary)
                            if offline.next != nil {
                                Button("Try Now") { tries += 1 }
                                    .buttonStyle(.borderless)
                            } else {
                                ProgressView().controlSize(.small)
                                Text("Trying again…").foregroundStyle(.tertiary)
                            }
                        }
                    }
                    .font(.footnote.weight(.medium))
                    .padding(.horizontal, 14)
                    .padding(.vertical, 7)
                    .glassEffect(.regular, in: .capsule)
                    .padding(.vertical, 6)
                    .animation(.default, value: offline.next == nil)
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
        if settings { return store.settingsItem(route.screen)?.title ?? String(localized: "Settings") }
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
    var settings: Bool
    var dismiss: ((Any?) -> Void)?
    var store: Store
    @Binding var title: String?
    @Binding var loaded: Bool
    @Binding var failure: String?
    @Binding var offline: Offline?
    var tries: Int
    @Environment(\.dynamicTypeSize) private var typeSize

    func makeCoordinator() -> Coordinator { Coordinator(view: self) }

    #if os(macOS)
    func makeNSView(context: Context) -> WKWebView { make(context) }
    func updateNSView(_ web: WKWebView, context: Context) { update(context) }
    static func dismantleNSView(_ web: WKWebView, coordinator: Coordinator) { coordinator.stop() }
    #else
    func makeUIView(context: Context) -> WKWebView { make(context) }
    func updateUIView(_ web: WKWebView, context: Context) {
        update(context)
        web.evaluateJavaScript(zoomScript)
    }
    static func dismantleUIView(_ web: WKWebView, coordinator: Coordinator) { coordinator.stop() }
    #endif

    private func update(_ context: Context) {
        let tried = context.coordinator.view.tries != tries
        context.coordinator.view = self
        if tried { Task { context.coordinator.tryAgain() } }
    }

    private func make(_ context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.userContentController.addScriptMessageHandler(context.coordinator, contentWorld: .page, name: "one")
        config.userContentController.addUserScript(WKUserScript(source: look, injectionTime: .atDocumentStart, forMainFrameOnly: true))
        // What this shell can do beyond the bridge's basics, for screens to offer it only where it works.
        config.userContentController.addUserScript(WKUserScript(source: "window.__oneShell = { scan: \(ShellFiles.canScan) };", injectionTime: .atDocumentStart, forMainFrameOnly: true))
        config.userContentController.addUserScript(WKUserScript(source: Self.noZoom, injectionTime: .atDocumentEnd, forMainFrameOnly: true))
        #if os(iOS)
        config.userContentController.addUserScript(WKUserScript(source: ShellMenus.script, injectionTime: .atDocumentStart, forMainFrameOnly: true))
        #endif
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
        context.coordinator.menus.web = web
        web.addInteraction(UIContextMenuInteraction(delegate: context.coordinator.menus))
        context.coordinator.menus.watch(web)
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
        \(raised)
        \(zoomScript)
        """
    }

    /// On an iPhone, a sheet is raised: its background is grey in the dark, and the kit's cards are lighter on it.
    private var raised: String {
        #if os(iOS)
        dismiss == nil ? "" : #"document.documentElement.setAttribute("data-raised", "");"#
        #else
        ""
        #endif
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
        #if os(iOS)
        let menus = ShellMenus()
        #endif

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

        /// The node's tries since it last answered, for the wait before the next: 2, 4, 8, 16, then every 30 seconds.
        private var misses = 0
        private var loading: Task<Void, Never>?

        func load(into web: WKWebView) {
            self.web = web
            let route = view.route, settings = view.settings, node = view.store.node
            loading?.cancel()
            loading = Task { [weak self] in
                do {
                    let page = try await settings ? node.settingsPage(route.screen, params: route.params) : node.page(app: route.app, screen: route.screen, params: route.params, scope: route.scope)
                    guard let self, !Task.isCancelled else { return }
                    // No base: a screen is an app's code, never at the node's address, where it would ask as this
                    // machine's own programs.
                    web.loadHTMLString(page, baseURL: nil)
                    ScreenCache.keep(page, for: route)
                    misses = 0
                    view.offline = nil
                } catch {
                    // Offline, the last copy of this screen stands in, saying so; its calls fail until the node answers,
                    // which it's asked again and again, waiting longer each time.
                    guard let self, !Task.isCancelled else { return }
                    let kept: Date
                    if let offline = view.offline {
                        kept = offline.kept
                    } else if let copy = ScreenCache.last(for: route) {
                        web.loadHTMLString(copy.html, baseURL: nil)
                        kept = copy.time
                    } else {
                        view.failure = error.localizedDescription
                        return
                    }
                    misses += 1
                    let wait = Double(min(30, 1 << min(misses, 5)))
                    view.offline = Offline(kept: kept, next: .now + wait)
                    try? await Task.sleep(for: .seconds(wait))
                    guard !Task.isCancelled else { return }
                    tryAgain()
                }
            }
        }

        func tryAgain() {
            guard let web, view.offline != nil else { return }
            view.offline?.next = nil
            load(into: web)
        }

        func stop() { loading?.cancel() }

        #if os(iOS)
        private var host: UIView? { web }
        #else
        private var host: NSView? { web }
        #endif

        /// The bridge (src/screen/bridge.js's list; tests/contracts.rs holds this to it), and for Settings screens alone,
        /// `"api" | "signOut" | "appearance" | "device" | "request"`. A page only ever calls its own app, but may open
        /// another's screens.
        func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) async -> (Any?, String?) {
            guard let body = message.body as? [String: Any] else { return (nil, "Not a message") }
            let route = view.route, store = view.store
            let type = body["type"] as? String ?? ""
            if ["api", "signOut", "appearance", "device", "request", "cache"].contains(type) && !view.settings { return (nil, "Only Settings can do that.") }
            switch type {
            case "api":
                do {
                    return (try await store.node.api(body["method"] as? String ?? "GET", body["path"] as? String ?? "", body: body["body"]), nil)
                } catch {
                    return (nil, error.localizedDescription)
                }
            case "cache":
                let key = body["key"] as? String ?? ""
                guard body.keys.contains("value") else { return (ScreenCache.kept(key), nil) }
                ScreenCache.keep(body["value"], as: key)
                return (nil, nil)
            case "signOut":
                #if os(iOS)
                await Phone.shared.signOut()
                #else
                await Mac.shared.signOut()
                #endif
                return (nil, nil)
            case "appearance":
                if let value = (body["value"] as? String).flatMap(Appearance.init(rawValue:)) { UserDefaults.standard.set(value.rawValue, forKey: "appearance") }
                return (UserDefaults.standard.string(forKey: "appearance").flatMap(Appearance.init(rawValue:))?.rawValue ?? Appearance.automatic.rawValue, nil)
            case "device":
                #if os(iOS)
                return (["name": UIDevice.current.name, "signOut": true], nil)
                #else
                return (["name": displayName(store.device?.name ?? store.state?.machine.name ?? ""), "signOut": Auth.signedIn], nil)
                #endif
            case "request":
                do {
                    return (try await store.ask(scope: body["scope"] as? String ?? "", id: body["id"] as? String ?? ""), nil)
                } catch {
                    return (nil, error.localizedDescription)
                }
            case "call" where view.settings:
                return (nil, "Settings has no app to call.")
            case "call":
                do {
                    let input = try JSONSerialization.data(withJSONObject: body["input"] ?? [:], options: .fragmentsAllowed)
                    let result = try await view.store.node.call(app: route.app, op: body["op"] as? String ?? "", input: input)
                    return (try JSONSerialization.jsonObject(with: result, options: .fragmentsAllowed), nil)
                } catch {
                    return (nil, error.localizedDescription)
                }
            case "open":
                // One of the client's own screens (Settings', Services, a request) from one of its own; else an app's.
                let params = (try? JSONSerialization.data(withJSONObject: body["params"] ?? [:])).map { String(decoding: $0, as: UTF8.self) } ?? "{}"
                let other = (body["app"] as? String).flatMap { $0.isEmpty ? nil : $0 }
                let screen = body["screen"] as? String ?? "main"
                let scope = (body["scope"] as? String).flatMap { $0.isEmpty ? nil : $0 } ?? route.scope
                let to: Route = if view.settings, other == nil { .setting(screen, params: params) } else { .screen(ScreenRoute(app: other ?? route.app, screen: screen, params: params, scope: scope)) }
                #if os(iOS)
                if body["sheet"] as? Bool == true { return (await store.present(to), nil) }
                // From a sheet, the sheet goes and it opens over the pages.
                view.dismiss?(nil)
                #endif
                store.path.append(to)
                return (nil, nil)
            case "close":
                if let dismiss = view.dismiss { dismiss(body["value"].flatMap { $0 is NSNull ? nil : $0 }) } else if !store.path.isEmpty { store.path.removeLast() }
                return (nil, nil)
            case "menu":
                let items = ShellMenuItem.from(body["items"])
                let point = CGPoint(x: body["x"] as? Double ?? 0, y: body["y"] as? Double ?? 0)
                let rect = (body["rect"] as? [Double]).flatMap { $0.count == 4 ? CGRect(x: $0[0], y: $0[1], width: $0[2], height: $0[3]) : nil }
                #if os(iOS)
                return (await menus.show(items, at: point, from: rect), nil)
                #else
                return (shellMenu(items, at: point, in: web), nil)
                #endif
            case "alert":
                let action = (body["action"] as? String).flatMap { $0.isEmpty ? nil : $0 }
                return (await shellAlert(title: body["title"] as? String ?? "", text: body["text"] as? String ?? "", action: action,
                                         danger: body["danger"] as? Bool ?? false, from: web), nil)
            case "link":
                if let url = URL(string: body["url"] as? String ?? ""), ["http", "https", "mailto", "tel"].contains(url.scheme ?? "") { Self.openOutside(url) }
                return (nil, nil)
            #if os(iOS)
            case "held":
                menus.held(body["rect"])
                return (nil, nil)
            #endif
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
                // Asks for what the app declares, in the Vault's sheet.
                let scope = route.scope ?? "user"
                do {
                    guard let id = try await store.node.requestItem(scope: scope, app: route.app, need: body["need"] as? String ?? "") else {
                        return (nil, "The Vault didn’t take the request")
                    }
                    return (try await store.ask(scope: scope, id: id), nil)
                } catch {
                    return (nil, error.localizedDescription)
                }
            case "title":
                view.title = body["text"] as? String
                return (nil, nil)
            case "icons":
                let names = body["names"] as? [String] ?? []
                return (Dictionary(names.compactMap { name in Icons.paths[name].map { (name, $0) } }) { a, _ in a }, nil)
            case "icon":
                let tile = AppTile(icon: body["icon"] as? String, seed: body["seed"] as? String ?? "", color: (body["color"] as? String).flatMap { $0.isEmpty ? nil : $0 },
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
                // A hold's haptic is the system's own, as it lifts.
                if menus.holding { return (nil, nil) }
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

/// What a shell shows for a route: a page, an app's screen, Settings or one of its screens, or a page's Services.
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
        case .setting(let name, let params):
            // An item with an app (the App Store) opens that app.
            if let app = store.settingsItem(name)?.app {
                AppScreen(route: ScreenRoute(app: app)).id(app)
            } else {
                let screen = ScreenRoute(app: "", screen: name, params: params)
                AppScreen(route: screen, settings: true).id(screen)
            }
        case .services(let scope):
            if let page = store.page(scope) { ServicesView(page: page, open: open) }
        }
    }
}

extension Store {
    /// A request for a Vault item, in the Vault's sheet: whether it was allowed, once the sheet is gone, or once it's
    /// answered somewhere else (another device, an agent's CLI), which closes the sheet.
    func ask(scope: String, id: String) async throws -> Bool {
        #if SCREENS
        // The request's screen, every client's, which closes once it's answered here or anywhere else.
        let params = (try? JSONSerialization.data(withJSONObject: ["scope": scope, "id": id])).map { String(decoding: $0, as: UTF8.self) } ?? "{}"
        _ = await present(.setting("request", params: params))
        return try await node.vaultRequests(scope: scope, id: id).first?.state == "set"
        #else
        let sheet = Sheet.vault(scope: scope, id: id)
        self.sheet = sheet
        var waited = 0
        while self.sheet == sheet {
            try? await Task.sleep(for: .milliseconds(300))
            waited += 1
            if waited % 7 == 0, let state = try? await node.vaultRequests(scope: scope, id: id).first?.state, state != "waiting", self.sheet == sheet {
                self.sheet = nil
            }
        }
        return try await node.vaultRequests(scope: scope, id: id).first?.state == "set"
        #endif
    }

    #if os(iOS)
    /// Slides a screen up over the others, as the bridge's `open` with `sheet` does: once it's gone, what it gave
    /// `one.close`.
    func present(_ route: Route) async -> Any? {
        let sheet = ScreenSheet(route: route)
        screenSheets.append(sheet)
        while screenSheets.contains(where: { $0 === sheet }) { try? await Task.sleep(for: .milliseconds(50)) }
        return sheet.value
    }

    /// Closes a sheet, with what it gave, and any slid up over it.
    func dismiss(_ sheet: ScreenSheet, value: Any? = nil) {
        guard let at = screenSheets.firstIndex(where: { $0 === sheet }) else { return }
        sheet.value = value
        screenSheets.removeSubrange(at...)
    }
    #endif

    /// The device this app talks to: this Mac's own, or the one a phone reaches.
    var device: Device? {
        let host = state?.host ?? state?.machine
        return install?.devices.first { $0.key == host?.key }
    }
}

/// The last copy of each screen this device drew, by route, for when the node can't be reached.
/// A screen shown from its last copy: when that was kept, and when the node is next asked for it (nil while it's being asked).
struct Offline {
    var kept: Date
    var next: Date?
}

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

    /// What the client's own screens keep between launches (`one.cache`), by key.
    private static func file(_ key: String) -> URL? {
        guard (1...40).contains(key.count), key.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "-") }) else { return nil }
        return dir.appending(path: "kept-\(key).json")
    }

    static func keep(_ value: Any?, as key: String) {
        guard let url = file(key), let data = try? JSONSerialization.data(withJSONObject: value ?? NSNull(), options: .fragmentsAllowed) else { return }
        try? data.write(to: url, options: .atomic)
    }

    static func kept(_ key: String) -> Any? {
        guard let url = file(key), let data = try? Data(contentsOf: url) else { return nil }
        return try? JSONSerialization.jsonObject(with: data, options: .fragmentsAllowed)
    }

    static func forget() {
        try? FileManager.default.removeItem(at: dir)
    }
}

#if os(iOS)
extension View {
    /// The screens slid up over this one (`Store.screenSheets`) from `level` on, each presenting the next.
    func screenSheets(from level: Int = 0) -> some View { modifier(ScreenSheets(level: level)) }
}

private struct ScreenSheets: ViewModifier {
    @Environment(Store.self) private var store
    var level: Int

    func body(content: Content) -> some View {
        content.sheet(item: Binding(
            get: { store.screenSheets.indices.contains(level) ? store.screenSheets[level] : nil },
            set: { if $0 == nil, store.screenSheets.indices.contains(level) { store.dismiss(store.screenSheets[level]) } }
        )) { sheet in
            ScreenSheetView(sheet: sheet, level: level)
        }
    }
}

/// A screen in a sheet: its title, and Done, or Cancel for a request, which closes itself once it's answered.
private struct ScreenSheetView: View {
    @Environment(Store.self) private var store
    var sheet: ScreenSheet
    var level: Int

    var body: some View {
        NavigationStack {
            screen
                .navigationBarTitleDisplayMode(.inline)
                .background(Color(.systemGroupedBackground))
                .toolbar {
                    if case .setting("request", _) = sheet.route {
                        ToolbarItem(placement: .cancellationAction) { Button("Cancel") { store.dismiss(sheet) } }
                    } else {
                        ToolbarItem(placement: .confirmationAction) { Button("Done") { store.dismiss(sheet) } }
                    }
                }
        }
        .screenSheets(from: level + 1)
    }

    @ViewBuilder private var screen: some View {
        switch sheet.route {
        case .setting(let name, let params):
            AppScreen(route: ScreenRoute(app: "", screen: name, params: params), settings: true) { store.dismiss(sheet, value: $0) }
        case .screen(let route):
            AppScreen(route: route) { store.dismiss(sheet, value: $0) }
        case .page, .settings, .services:
            EmptyView()
        }
    }
}
#endif
