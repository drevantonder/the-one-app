import SwiftUI
import WidgetKit

/// Settings' place among the pages; no scope is named this. A link may turn the pages to it (`store.current`).
let settingsID = "settings"

/// The pages side by side, then Settings: swipe between them, scroll within one. At the bottom, a dot per page, with a
/// cog as Settings' dot, and the current page's name above them; the pages have no titles of their own.
struct PagesView: View {
    @Environment(Store.self) private var store
    @Environment(\.scenePhase) private var scenePhase
    /// The page scrolled to: a page's scope, kept in step with the store's current page, or Settings.
    @State private var position: String?

    var body: some View {
        @Bindable var store = store
        NavigationStack(path: $store.path) {
            ScrollView(.horizontal) {
                LazyHStack(spacing: 0) {
                    ForEach(store.pages) { page in
                        PageView(page: page, open: open).containerRelativeFrame(.horizontal).id(page.scope)
                    }
                    SettingsPage(open: open).containerRelativeFrame(.horizontal).id(settingsID)
                }
                .scrollTargetLayout()
            }
            .scrollTargetBehavior(.paging)
            .scrollIndicators(.hidden)
            .scrollPosition(id: $position)
            .scrollClipDisabled()
            .onChange(of: position) { if let position, position != settingsID, position != store.current { store.current = position } }
            .onChange(of: store.current, initial: true) { if position != store.current { withAnimation(.snappy) { position = store.current } } }
            .safeAreaBar(edge: .bottom) { BottomBar(position: $position).padding(.bottom, 2) }
            // As on the Home Screen, Edit and Done take the status bar's place, either side of the Dynamic Island.
            .overlay(alignment: .top) {
                if store.editing && position != settingsID { EditBar(page: store.page(store.current)).padding(.top, 14).ignoresSafeArea(edges: .top) }
            }
            .statusBarHidden(store.editing)
            .sensoryFeedback(.impact(weight: .medium), trigger: store.editing) { _, new in new }
            .toolbar(.hidden, for: .navigationBar)
            .navigationDestination(for: Route.self) { route in
                RouteView(route: route, open: open)
                    .navigationBarTitleDisplayMode(.inline)
                    .background(Color(.systemGroupedBackground))
            }
            .overlay {
                if store.install?.joining == true {
                    ProgressView("Joining…").padding(24).background(.regularMaterial, in: .rect(cornerRadius: 18))
                } else if store.state == nil, case .unreachable(let message) = store.phase {
                    ContentUnavailableView {
                        Label("Can’t Reach Your Servers", systemImage: "bolt.horizontal.circle")
                    } description: {
                        Text(message)
                    } actions: {
                        Text("It keeps trying.").foregroundStyle(.secondary)
                    }
                    .background(.regularMaterial)
                } else if store.state == nil {
                    ProgressView()
                }
            }
        }
        .sheet(item: $store.sheet) { sheet in
            switch sheet {
            case .pair: PairSheet()
            case .pages: PagesEditor()
            case .addWidget(let scope): AddWidgetSheet(scope: scope)
            case .join, .connectors: EmptyView()
            case .vault(let scope, let id): VaultRequestSheet(scope: scope, id: id)
            }
        }
        .onChange(of: store.pages) {
            if store.current != settingsID, !store.pages.contains(where: { $0.scope == store.current }) { store.current = "user" }
        }
        .onChange(of: store.widgets, initial: true) { Task { await store.saveForHomeScreen() } }
        .onChange(of: scenePhase) {
            if scenePhase == .active { Task { try? await store.refreshState(); await store.refreshApps() } }
            if scenePhase == .background { store.editing = false }
        }
    }

    private func open(_ route: Route) { store.path.append(route) }
}

/// A dot per page, the last a cog for Settings, in a glass capsule that stays the same size, with the current page's
/// name above it: tap the dots to list the pages, tap the cog for Settings, or swipe along the bar to change page.
private struct BottomBar: View {
    @Environment(Store.self) private var store
    @Binding var position: String?
    @ScaledMetric(relativeTo: .headline) private var height: CGFloat = 44
    /// Where a swipe along the bar started, while it lasts.
    @State private var swipeStart: Int?

    var body: some View {
        let pages = store.pages
        let positions = pages.map(\.scope) + [settingsID]
        let index = positions.firstIndex { $0 == position } ?? 0
        let onSettings = positions[index] == settingsID
        let title = onSettings ? String(localized: "Settings") : pages.indices.contains(index) ? pages[index].title : ""
        let go = { (i: Int) in if positions.indices.contains(i) { withAnimation(.snappy) { position = positions[i] } } }

        VStack(spacing: 6) {
            Text(title)
                .font(.footnote.weight(.semibold))
                .foregroundStyle(.secondary)
                // A long name shrinks, then wraps, rather than lose a word.
                .lineLimit(1...2)
                .minimumScaleFactor(0.75)
                .multilineTextAlignment(.center)
                .contentTransition(.opacity)
                .padding(.horizontal, 40)
                .accessibilityHidden(true)

            HStack(spacing: 0) {
                Menu {
                    Picker("Pages", selection: Binding(get: { position ?? "" }, set: { page in withAnimation(.snappy) { position = page } })) {
                        ForEach(pages) { Text($0.title).tag($0.scope) }
                        Text("Settings").tag(settingsID)
                    }
                    .pickerStyle(.inline)
                    Divider()
                    Button("Edit Pages…", systemImage: "list.bullet") { store.sheet = .pages }
                    if !onSettings {
                        Button("Edit Home Page", systemImage: "square.grid.3x3") { withAnimation { store.editing = true } }
                    }
                } label: {
                    HStack(spacing: 6) {
                        ForEach(pages.indices, id: \.self) { i in
                            Capsule()
                                .fill(i == index ? AnyShapeStyle(.primary) : AnyShapeStyle(.tertiary))
                                .frame(width: i == index ? 16 : 7, height: 7)
                        }
                    }
                    .padding(.leading, 18)
                    .padding(.trailing, 6)
                    .frame(height: height)
                    .contentShape(.rect)
                }
                .menuOrder(.fixed)
                .buttonStyle(.plain)
                .accessibilityLabel(title)
                .accessibilityValue(Text("Page \(index + 1) of \(positions.count)"))
                .accessibilityHint("Shows every page")
                .accessibilityAdjustableAction { direction in go(direction == .increment ? index + 1 : index - 1) }

                Button { go(positions.count - 1) } label: {
                    Image(systemName: "gearshape.fill")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(onSettings ? AnyShapeStyle(.primary) : AnyShapeStyle(.tertiary))
                        .overlay(alignment: .topTrailing) {
                            if store.hasRequests { Circle().fill(.red).frame(width: 7, height: 7).offset(x: 3, y: -3) }
                        }
                        .padding(.trailing, 18)
                        .frame(height: height)
                        .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Settings")
                .accessibilityValue(store.hasRequests ? Text("Requests waiting") : Text(""))
            }
            .glassEffect(.regular.interactive(), in: .capsule)
        }
        .animation(.snappy, value: index)
        .contentShape(.rect)
        .highPriorityGesture(
            // A page per 40 points, as a finger slides along the bar.
            DragGesture(minimumDistance: 12)
                .onChanged { drag in
                    let start = swipeStart ?? index
                    swipeStart = start
                    let target = min(max(start - Int(drag.translation.width / 40), 0), positions.count - 1)
                    if target != index { go(target) }
                }
                .onEnded { _ in swipeStart = nil }
        )
        .sensoryFeedback(.selection, trigger: index) { _, _ in swipeStart != nil }
        .frame(maxWidth: .infinity)
    }
}

extension Store {
    /// Leaves every page's widgets, and the images they show, where the home-screen widgets read them, then has
    /// those draw again.
    func saveForHomeScreen() async {
        let items = pages.flatMap { page in
            widgets(on: page).map { w in
                WidgetCache.Item(app: w.app, widget: w.widget, scope: page.scope, page: page.title, title: w.title ?? app(w.app)?.label ?? w.widget,
                                 icon: symbol(app(w.app)?.icon), error: w.error, root: w.root, sizes: w.sizes)
            }
        }
        guard !items.isEmpty else { return }
        WidgetCache(install: install?.name, widgets: items).save()
        WidgetCenter.shared.reloadAllTimelines()
        let images = items.flatMap { [$0.root] + ($0.sizes ?? [:]).values }.flatMap(\.imageURLs)
        if !images.isEmpty {
            await WidgetImages.save(images)
            WidgetCenter.shared.reloadAllTimelines()
        }
    }
}
