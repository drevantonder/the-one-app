#if SCREENS
import SwiftUI

/// The Screens build's home (a build of its own, beside the iPhone app, to try them by hand): your pages as every
/// client's screen draws them (src/screen/pages.html), swiped between in one web view, with the iPhone's own menus,
/// alerts and sheets answering it; what they open pushed over them with Back. Signing in, the Keychain and push are
/// the iPhone app's.
struct ScreensHome: View {
    @Environment(Store.self) private var store
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        @Bindable var store = store
        NavigationStack(path: $store.path) {
            AppScreen(route: ScreenRoute(app: "", screen: "pages", params: pages), settings: true)
                .id(store.current)
                .ignoresSafeArea(edges: .bottom)
                .background(Color(.systemGroupedBackground))
                .toolbar(.hidden, for: .navigationBar)
                .navigationDestination(for: Route.self) { route in
                    RouteView(route: route) { store.path.append($0) }
                        .navigationBarTitleDisplayMode(.inline)
                        .background(Color(.systemGroupedBackground))
                }
                .overlay {
                    if store.state == nil, case .unreachable(let message) = store.phase {
                        ContentUnavailableView {
                            Label("Can’t Reach Your Devices", systemImage: "bolt.horizontal.circle")
                        } description: {
                            Text(message)
                        } actions: {
                            Text("It keeps trying.").foregroundStyle(.secondary)
                        }
                        .background(.regularMaterial)
                    }
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
        .onChange(of: scenePhase) {
            if scenePhase == .active { Task { try? await store.refreshState(); await store.refreshApps() } }
        }
    }

    /// Every page side by side, then Settings, opened at the one a link turned to.
    private var pages: String {
        let params: [String: Any] = ["swipe": true, "scope": store.current]
        return (try? JSONSerialization.data(withJSONObject: params)).map { String(decoding: $0, as: UTF8.self) } ?? "{}"
    }
}
#endif
