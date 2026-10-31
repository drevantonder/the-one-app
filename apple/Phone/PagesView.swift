import SwiftUI
import WidgetKit

/// The pages, side by side: swipe between them, scroll within one. A pill of their names sits at the bottom.
struct PagesView: View {
    @Environment(Store.self) private var store
    @Environment(\.scenePhase) private var scenePhase
    /// The page scrolled to, kept in step with the store's current page.
    @State private var position: String?

    var body: some View {
        @Bindable var store = store
        NavigationStack(path: $store.path) {
            ScrollView(.horizontal) {
                LazyHStack(spacing: 0) {
                    ForEach(store.pages) { page in
                        PageView(page: page, open: open).containerRelativeFrame(.horizontal).id(page.scope)
                    }
                    SettingsPage(open: open).containerRelativeFrame(.horizontal).id("settings")
                }
                .scrollTargetLayout()
            }
            .scrollTargetBehavior(.paging)
            .scrollIndicators(.hidden)
            .scrollPosition(id: $position)
            .scrollClipDisabled()
            .onChange(of: position) { if let position, position != store.current { store.current = position } }
            .onChange(of: store.current, initial: true) { if position != store.current { withAnimation(.snappy) { position = store.current } } }
            .safeAreaInset(edge: .bottom) { PagePill().padding(.bottom, 2) }
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
            case .join: EmptyView()
            }
        }
        .onChange(of: store.pages) {
            if store.current != "settings" && !store.pages.contains(where: { $0.scope == store.current }) { store.current = "user" }
        }
        .onChange(of: store.allWidgets, initial: true) {
            guard !store.allWidgets.isEmpty else { return }
            WidgetCache(install: store.install?.name, widgets: store.allWidgets.map { w in
                WidgetCache.Item(title: w.title ?? store.app(w.app)?.label ?? "", icon: symbol(store.app(w.app)?.icon), root: w.root)
            }).save()
            WidgetCenter.shared.reloadAllTimelines()
        }
        .onChange(of: scenePhase) {
            if scenePhase == .active { Task { try? await store.refreshState(); await store.refreshApps() } }
        }
    }

    private func open(_ route: Route) { store.path.append(route) }
}

/// The pages' names, the current one highlighted; it scrolls sideways when they don't fit. Holding it edits them.
private struct PagePill: View {
    @Environment(Store.self) private var store

    var body: some View {
        ViewThatFits(in: .horizontal) {
            items
            ScrollViewReader { proxy in
                ScrollView(.horizontal, showsIndicators: false) { items }
                    .frame(maxWidth: 340)
                    .onChange(of: store.current, initial: true) { withAnimation { proxy.scrollTo(store.current, anchor: .center) } }
            }
        }
        .glassEffect(.regular, in: .capsule)
        .padding(.horizontal, 16)
        .onLongPressGesture { store.sheet = .pages }
    }

    private var items: some View {
        HStack(spacing: 0) {
            ForEach(store.pages) { page in
                item(page.scope) { Text(page.title) }
                Text("·").foregroundStyle(.tertiary)
            }
            item("settings") {
                Image(systemName: "gearshape.fill")
                    .overlay(alignment: .topTrailing) {
                        if store.hasRequests { Circle().fill(.red).frame(width: 7, height: 7).offset(x: 3, y: -2) }
                    }
                    .accessibilityLabel("Settings")
            }
        }
        .font(.subheadline.weight(.semibold))
        .padding(4)
    }

    private func item(_ id: String, @ViewBuilder label: () -> some View) -> some View {
        let on = store.current == id
        return Button { withAnimation(.snappy) { store.current = id } } label: {
            label()
                .foregroundStyle(on ? AnyShapeStyle(.white) : AnyShapeStyle(.primary))
                .padding(.horizontal, 12)
                .padding(.vertical, 7)
                .background(on ? AnyShapeStyle(.tint) : AnyShapeStyle(.clear), in: .capsule)
                .contentShape(.capsule)
        }
        .buttonStyle(.plain)
        .id(id)
    }
}
