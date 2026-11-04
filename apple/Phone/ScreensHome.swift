import SwiftUI
import WidgetKit

/// The iPhone's home: your pages as every client's screen draws them (src/screen/pages.html), swiped between in one
/// web view, with the iPhone's own menus, alerts and sheets answering it; what they open pushed over them with Back.
/// Before the iPhone is let in, or can reach anything, it's the approval, joining or can't-reach screen instead.
struct ScreensHome: View {
    @Environment(Store.self) private var store
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        @Bindable var store = store
        NavigationStack(path: $store.path) {
            content
                .groupedBackground()
                .toolbar(.hidden, for: .navigationBar)
                .navigationDestination(for: Route.self) { route in
                    RouteView(route: route)
                        .navigationBarTitleDisplayMode(.inline)
                        .groupedBackground()
                }
        }
        .screenSheets()
        .sheet(item: $store.sheet) { sheet in
            // Push's key, asked for by the shell, is the iPhone app's own sheet.
            if case .ask(let ask) = sheet { VaultRequestSheet(scope: ask.scope, id: "", ask: ask) }
        }
        // A link's request for a Vault item is the request screen.
        .onChange(of: store.sheet) {
            guard case .vault(let scope, let id) = store.sheet else { return }
            store.sheet = nil
            Task { _ = try? await store.ask(scope: scope, id: id) }
        }
        // The widgets, as the home screen's read them, and the same widgets fetched again: the home screen's stop
        // saying they're old, without redrawing at every fetch.
        .onChange(of: store.widgets, initial: true) { Task { await store.saveForHomeScreen() } }
        .onChange(of: store.widgetsFetched) {
            guard let fetched = store.widgetsFetched, fetched.timeIntervalSince(WidgetCache.load()?.fetched ?? .distantPast) > 600 else { return }
            Task { await store.saveForHomeScreen() }
        }
        .onChange(of: scenePhase) {
            if scenePhase == .active { Task { try? await store.refreshState(); await store.refreshApps() } }
        }
    }

    @ViewBuilder private var content: some View {
        if case .joining(let code) = store.phase {
            ApproveView(code: code) { Task { await Phone.shared.signOut() } }
        } else if store.install?.joining == true {
            ProgressView("Joining…").padding(24).background(.regularMaterial, in: .rect(cornerRadius: 18))
        } else if store.state == nil {
            // A spinner only after a moment; can't reach only once it's been trying a while, not a first miss.
            ZStack {
                Later(.milliseconds(400)) { ProgressView() }
                Later(.seconds(3)) {
                    if case .unreachable(let message) = store.phase {
                        ContentUnavailableView {
                            Label("Can’t Reach Your Devices", systemImage: "bolt.horizontal.circle")
                        } description: {
                            Text(message)
                        } actions: {
                            Text("Trying again…").foregroundStyle(.secondary)
                        }
                    }
                }
            }
        } else {
            AppScreen(route: ScreenRoute(app: "", screen: "pages", params: pages), settings: true)
                .id(store.current)
                .ignoresSafeArea(edges: .bottom)
        }
    }

    /// Every page side by side, then Settings, opened at the one a link turned to.
    private var pages: String { Store.params(["swipe": true, "scope": store.current]) }
}

extension Store {
    /// Leaves every page's widgets, and the images they show, where the home-screen widgets read them, then has
    /// those draw again.
    func saveForHomeScreen() async {
        let items = pages.flatMap { page in
            widgets(on: page).map { w in
                WidgetCache.Item(app: w.app, widget: w.widget, scope: page.scope, page: page.title, title: w.title ?? app(w.app)?.label ?? w.widget,
                                 icon: app(w.app)?.icon ?? "", image: app(w.app)?.image, imageKind: app(w.app)?.imageKind, error: w.error, root: w.root, sizes: w.sizes)
            }
        }
        // Signed in again with no widgets, the home screen's stop saying to sign in.
        guard Phone.nodeURL != nil || Auth.signedIn, !items.isEmpty || WidgetCache.load()?.signedOut == true else { return }
        let trees = items.flatMap { [$0.root] + ($0.sizes ?? [:]).values }
        Icons.keep(Set(items.map(\.icon) + trees.flatMap(\.iconNames)))
        WidgetCache(install: install?.name, widgets: items, fetched: widgetsFetched).save()
        WidgetCenter.shared.reloadAllTimelines()
        let images = trees.flatMap(\.imageURLs)
        if !images.isEmpty {
            await WidgetImages.save(images)
            WidgetCenter.shared.reloadAllTimelines()
        }
    }
}
