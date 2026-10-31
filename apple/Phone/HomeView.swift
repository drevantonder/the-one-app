import SwiftUI
import WidgetKit

/// The space: the apps it has screens for, then its devices.
struct HomeView: View {
    @Environment(Store.self) private var store
    @Environment(\.scenePhase) private var scenePhase
    @State private var settings = false

    var body: some View {
        @Bindable var store = store
        NavigationStack(path: $store.path) {
            Group {
                if let space = store.space {
                    List {
                        if !store.screenApps.isEmpty {
                            Section {
                                ForEach(store.screenApps) { app in
                                    NavigationLink(value: store.main(app)) { AppRow(app: app) }
                                }
                            }
                        }
                        Section {
                            NavigationLink(value: Route.devices) {
                                LabeledContent {
                                    Text("\(space.devices.filter(\.online).count) of \(space.devices.count) online")
                                } label: {
                                    Label("Devices", systemImage: "laptopcomputer.and.iphone")
                                }
                            }
                        }
                    }
                } else if case .unreachable(let message) = store.phase {
                    ContentUnavailableView {
                        Label("Can’t Reach Your Space", systemImage: "bolt.horizontal.circle")
                    } description: {
                        Text(message)
                    } actions: {
                        Text("It keeps trying.").foregroundStyle(.secondary)
                    }
                } else {
                    ProgressView()
                }
            }
            .navigationTitle(store.space?.title ?? "")
            .navigationBarTitleDisplayMode(.large)
            .refreshable { await refresh() }
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Settings", systemImage: "gearshape") { settings = true }
                }
            }
            .navigationDestination(for: Route.self) { route in
                RouteView(route: route)
                    .navigationBarTitleDisplayMode(.inline)
                    .background(Color(.systemGroupedBackground))
            }
        }
        .sheet(isPresented: $settings) { SettingsView() }
        .onChange(of: store.widgets, initial: true) {
            guard !store.widgets.isEmpty else { return }
            WidgetCache(space: store.space?.title, widgets: store.widgets.map { w in
                WidgetCache.Item(title: w.title ?? store.app(w.app)?.label ?? "", icon: symbol(store.app(w.app)?.icon), root: w.root)
            }).save()
            WidgetCenter.shared.reloadAllTimelines()
        }
        .onChange(of: scenePhase) { if scenePhase == .active { Task { await refresh() } } }
    }

    private func refresh() async {
        try? await store.refreshState()
        await store.refreshSpace()
    }
}

/// An app, with its first widget's headline under its name.
private struct AppRow: View {
    @Environment(Store.self) private var store
    var app: AppInfo

    var body: some View {
        let detail = store.widgets.first { $0.app == app.name }?.root.headline
        HStack(spacing: 12) {
            Image(systemName: symbol(app.icon))
                .font(.system(size: 16, weight: .semibold))
                .foregroundStyle(.white)
                .frame(width: 36, height: 36)
                .background(.tint, in: .circle)
            VStack(alignment: .leading, spacing: 2) {
                Text(app.label)
                    .font(.body.weight(.semibold))
                if let detail {
                    Text(detail)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
            }
        }
        .padding(.vertical, 2)
    }
}
