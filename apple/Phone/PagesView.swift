import SwiftUI
import WidgetKit

/// The pages, side by side: swipe between them, scroll within one. At the bottom, the current page's name and a dot
/// per page, and the cog beside them.
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
                }
                .scrollTargetLayout()
            }
            .scrollTargetBehavior(.paging)
            .scrollIndicators(.hidden)
            .scrollPosition(id: $position)
            .scrollClipDisabled()
            .onChange(of: position) { if let position, position != store.current { store.current = position } }
            .onChange(of: store.current, initial: true) { if position != store.current { withAnimation(.snappy) { position = store.current } } }
            .safeAreaInset(edge: .bottom) { BottomBar(open: open).padding(.bottom, 2) }
            .toolbar(.hidden, for: .navigationBar)
            .navigationDestination(for: Route.self) { route in
                RouteView(route: route, open: open)
                    .navigationBarTitleDisplayMode(route == .settings ? .large : .inline)
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
            if !store.pages.contains(where: { $0.scope == store.current }) { store.current = "user" }
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

/// The current page's name and its dots, in a glass capsule that lists every page; the cog in a glass circle beside it.
private struct BottomBar: View {
    @Environment(Store.self) private var store
    var open: (Route) -> Void
    @ScaledMetric(relativeTo: .headline) private var height: CGFloat = 50

    var body: some View {
        GlassEffectContainer {
            HStack(spacing: 12) {
                Color.clear.frame(width: height, height: height)
                Spacer(minLength: 0)
                PageMenu(height: height)
                Spacer(minLength: 0)
                Button { open(.settings) } label: {
                    Image(systemName: "gearshape")
                        .font(.title3)
                        .overlay(alignment: .topTrailing) {
                            if store.hasRequests { Circle().fill(.red).frame(width: 9, height: 9).offset(x: 5, y: -4) }
                        }
                        .frame(width: height, height: height)
                        .contentShape(.circle)
                }
                .buttonStyle(.plain)
                .glassEffect(.regular.interactive(), in: .circle)
                .accessibilityLabel("Settings")
                .accessibilityValue(store.hasRequests ? Text("Requests waiting") : Text(""))
            }
            .padding(.horizontal, 20)
        }
    }
}

/// The capsule: tapping it lists the pages to jump to, and edits them.
private struct PageMenu: View {
    @Environment(Store.self) private var store
    var height: CGFloat

    var body: some View {
        let pages = store.pages
        let index = pages.firstIndex { $0.scope == store.current } ?? 0
        Menu {
            Picker("Pages", selection: Binding(get: { store.current }, set: { page in withAnimation(.snappy) { store.current = page } })) {
                ForEach(pages) { Text($0.title).tag($0.scope) }
            }
            .pickerStyle(.inline)
            Divider()
            Button("Edit Pages…", systemImage: "list.bullet") { store.sheet = .pages }
        } label: {
            HStack(spacing: 10) {
                Text(pages.indices.contains(index) ? pages[index].title : "")
                    .font(.headline)
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                if pages.count > 1 {
                    HStack(spacing: 6) {
                        ForEach(pages.indices, id: \.self) { i in
                            Capsule()
                                .fill(i == index ? AnyShapeStyle(.primary) : AnyShapeStyle(.tertiary))
                                .frame(width: i == index ? 16 : 7, height: 7)
                        }
                    }
                    .animation(.snappy, value: index)
                }
            }
            .padding(.horizontal, 20)
            .frame(height: height)
            .contentShape(.capsule)
        }
        .menuOrder(.fixed)
        .buttonStyle(.plain)
        .glassEffect(.regular.interactive(), in: .capsule)
        .accessibilityLabel(pages.indices.contains(index) ? pages[index].title : "")
        .accessibilityValue(Text("Page \(index + 1) of \(pages.count)"))
        .accessibilityHint("Shows every page")
        .accessibilityAdjustableAction { direction in
            let next = direction == .increment ? index + 1 : index - 1
            if pages.indices.contains(next) { withAnimation(.snappy) { store.current = pages[next].scope } }
        }
    }
}
