import SwiftUI
import WebKit

/// An app's screen: the page the node serves for it, in a web view that answers its bridge (SPEC.md "Screens").
struct AppScreen: View {
    @Environment(Store.self) private var store
    var route: ScreenRoute
    @State private var title: String?
    @State private var loaded = false
    @State private var slow = false
    @State private var failure: String?

    var body: some View {
        ScreenWebView(route: route, store: store, title: $title, loaded: $loaded, failure: $failure)
            .opacity(loaded ? 1 : 0)
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
            .navigationTitle(title ?? defaultTitle)
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
    @Environment(\.dynamicTypeSize) private var typeSize

    func makeCoordinator() -> Coordinator { Coordinator(view: self) }

    #if os(macOS)
    func makeNSView(context: Context) -> WKWebView { make(context) }
    func updateNSView(_ web: WKWebView, context: Context) { context.coordinator.view = self }
    #else
    func makeUIView(context: Context) -> WKWebView { make(context) }
    func updateUIView(_ web: WKWebView, context: Context) {
        context.coordinator.view = self
        web.pageZoom = zoom
    }
    #endif

    private func make(_ context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.userContentController.addScriptMessageHandler(context.coordinator, contentWorld: .page, name: "one")
        config.userContentController.addUserScript(WKUserScript(source: look, injectionTime: .atDocumentStart, forMainFrameOnly: true))
        config.userContentController.addUserScript(WKUserScript(source: Self.noZoom, injectionTime: .atDocumentEnd, forMainFrameOnly: true))
        let web = WKWebView(frame: .zero, configuration: config)
        web.navigationDelegate = context.coordinator
        web.allowsLinkPreview = false
        web.pageZoom = zoom
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
    private var zoom: CGFloat {
        #if os(macOS)
        1
        #else
        UIFontMetrics(forTextStyle: .body).scaledValue(for: 17, compatibleWith: UITraitCollection(preferredContentSizeCategory: UIContentSizeCategory(typeSize))) / 17
        #endif
    }

    /// What makes a page sit in the shell as its own: the shell's background shows through, its accent is the
    /// shell's in light and dark, and nothing zooms.
    private var look: String {
        let accent = store.brand?.accent ?? Bundle.main.object(forInfoDictionaryKey: "BrandAccent") as? String ?? ""
        return """
        const style = document.createElement("style");
        style.textContent = `
          html, body { background: transparent !important; }
          html { -webkit-tap-highlight-color: transparent; -webkit-touch-callout: none; }
          :root:root { --accent: \(brandCSS(accent, dark: false)); }
          @media (prefers-color-scheme: dark) { :root:root { --accent: \(brandCSS(accent, dark: true)); } }
        \(Self.platformCSS)`;
        document.documentElement.appendChild(style);
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
    final class Coordinator: NSObject, WKScriptMessageHandlerWithReply, WKNavigationDelegate {
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
                    let page = try await node.page(app: route.app, screen: route.screen, params: route.params)
                    web.loadHTMLString(page.html, baseURL: page.base)
                } catch {
                    view.failure = error.localizedDescription
                }
            }
        }

        /// The bridge: `{ type: "call" | "open" | "title", … }`. A page only ever calls its own app.
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
                view.store.path.append(.screen(ScreenRoute(app: route.app, screen: body["screen"] as? String ?? "main", params: params)))
                return (nil, nil)
            case "title":
                view.title = body["text"] as? String
                return (nil, nil)
            default:
                return (nil, "Unknown message")
            }
        }

        func webView(_ web: WKWebView, decidePolicyFor action: WKNavigationAction) async -> WKNavigationActionPolicy {
            // Links leave for the browser; the page itself stays put.
            guard action.navigationType == .linkActivated, let url = action.request.url else { return .allow }
            #if os(macOS)
            NSWorkspace.shared.open(url)
            #else
            await UIApplication.shared.open(url)
            #endif
            return .cancel
        }

        func webView(_ web: WKWebView, didFinish navigation: WKNavigation!) {
            view.loaded = true
        }

        func webViewWebContentProcessDidTerminate(_ web: WKWebView) {
            load(into: web)
        }
    }
}

/// What a shell shows for a route: an app's screen, or one of its own views.
struct RouteView: View {
    var route: Route

    var body: some View {
        switch route {
        case .screen(let screen): AppScreen(route: screen).id(screen)
        case .devices: DevicesView()
        }
    }
}
