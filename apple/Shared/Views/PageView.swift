import SwiftUI

#if os(macOS)
private let iconSize: CGFloat = 60
#else
@MainActor private let iconSize: CGFloat = UIDevice.current.userInterfaceIdiom == .pad ? 76 : 62
#endif

/// A page: its title, the widgets of the apps installed in its scope, then their icons. Wide, the widgets take
/// a column on the left and the icons the rest.
struct PageView: View {
    @Environment(Store.self) private var store
    var page: Page
    var open: (Route) -> Void
    @State private var width: CGFloat = 0

    var body: some View {
        let apps = store.apps(on: page), widgets = store.widgets(on: page)
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                PageHeader(title: page.title, subtitle: page.subtitle)
                if apps.isEmpty && widgets.isEmpty {
                    EmptyPage(page: page, open: open)
                } else if width > 700 {
                    HStack(alignment: .top, spacing: 28) {
                        if !widgets.isEmpty {
                            WidgetColumn(widgets: widgets, page: page, open: open).frame(width: min(360, width * 0.4))
                        }
                        IconGrid(apps: apps, page: page, open: open)
                    }
                } else {
                    WidgetColumn(widgets: widgets, page: page, open: open)
                    IconGrid(apps: apps, page: page, open: open)
                }
            }
            .padding(.horizontal, 20)
            .padding(.bottom, 96)
            .frame(maxWidth: 1100, alignment: .leading)
            .frame(maxWidth: .infinity)
        }
        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
        .groupedBackground()
        .refreshable { try? await store.refreshState(); await store.refreshApps() }
    }
}

/// A page's title, large at its top. On a Mac it is the window's title too, though the toolbar doesn't repeat it.
struct PageHeader: View {
    var title: String
    var subtitle: String

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title)
                .font(.largeTitle.bold())
                .foregroundStyle(.primary)
            Text(subtitle)
                .font(.subheadline)
                .foregroundStyle(.secondary)
        }
        .padding(.top, 4)
        .padding(.horizontal, 4)
        #if os(macOS)
        .navigationTitle(title)
        .navigationSubtitle(subtitle)
        .toolbar(removing: .title)
        #endif
    }
}

private struct WidgetColumn: View {
    @Environment(Store.self) private var store
    var widgets: [WidgetInfo]
    var page: Page
    var open: (Route) -> Void

    var body: some View {
        VStack(spacing: 12) {
            ForEach(widgets) { widget in
                let app = store.app(widget.app)
                WidgetCard(widget: widget, app: app)
                    .onTapGesture { if let app, app.hasScreens { open(store.open(app, from: page)) } }
            }
        }
    }
}

/// A widget in a grouped container, under its app's icon and title.
struct WidgetCard: View {
    var widget: WidgetInfo
    var app: AppInfo?

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 7) {
                AppTile(symbol: symbol(app?.icon), seed: widget.app, size: 18)
                Text(widget.title ?? app?.label ?? widget.widget)
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            TimelineView(.everyMinute) { context in
                WidgetTree(node: widget.root, now: context.date)
                    .font(.subheadline)
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .groupedContainer()
        .contentShape(.rect)
    }
}

private struct IconGrid: View {
    @Environment(Store.self) private var store
    var apps: [AppInfo]
    var page: Page
    var open: (Route) -> Void
    @State private var error: String?

    var body: some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: iconSize + 18, maximum: iconSize + 34), spacing: 10, alignment: .top)],
                  alignment: .leading, spacing: 20) {
            ForEach(apps) { app in
                IconButton(title: app.label) { open(store.open(app, from: page)) } icon: {
                    AppTile(symbol: symbol(app.icon), seed: app.name, size: iconSize)
                }
                    .contextMenu {
                        if store.canArrange(page.scope) {
                            Button("Remove from \(page.title)", systemImage: "minus.circle", role: .destructive) { remove(app) }
                        }
                    }
            }
        }
        .alert("Couldn’t Remove It", isPresented: .constant(error != nil)) {
            Button("OK") { error = nil }
        } message: {
            Text(error ?? "")
        }
    }

    private func remove(_ app: AppInfo) {
        Task {
            do { try await store.change { try await $0.install(app: app.name, scope: page.scope, false) } } catch { self.error = error.localizedDescription }
        }
    }
}

/// A large rounded icon with its name under it, as a home screen's.
struct IconButton<Icon: View>: View {
    var title: String
    var badge = false
    var action: () -> Void
    @ViewBuilder var icon: Icon

    var body: some View {
        Button(action: action) {
            VStack(spacing: 6) {
                icon
                    .overlay(alignment: .topTrailing) {
                        if badge {
                            Circle().fill(.red).frame(width: 14, height: 14).offset(x: 4, y: -4)
                        }
                    }
                Text(title)
                    .font(.caption)
                    .lineLimit(1)
                    .foregroundStyle(.primary)
            }
            .frame(maxWidth: .infinity)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
    }
}

private struct EmptyPage: View {
    @Environment(Store.self) private var store
    var page: Page
    var open: (Route) -> Void

    var body: some View {
        VStack(spacing: 10) {
            Text("Nothing here yet")
                .font(.headline)
            Text(store.canArrange(page.scope)
                 ? "Apps you add to \(page.title) show up here, with their widgets above them."
                 : "When its admins add apps to \(page.title), they show up here.")
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            if store.canArrange(page.scope) {
                Button("Open the App Store") { open(.setting(.store)) }
                    .buttonStyle(.borderedProminent)
                    .padding(.top, 4)
            }
        }
        .frame(maxWidth: .infinity)
        .padding(28)
        .groupedContainer()
    }
}

/// The cog's page: the shell's own views, as icons. A phone pushes it, under its own navigation bar.
struct SettingsPage: View {
    @Environment(Store.self) private var store
    var open: (Route) -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                #if os(macOS)
                PageHeader(title: String(localized: "Settings"), subtitle: subtitle)
                #endif
                LazyVGrid(columns: [GridItem(.adaptive(minimum: iconSize + 18, maximum: iconSize + 34), spacing: 10, alignment: .top)],
                          alignment: .leading, spacing: 20) {
                    ForEach(store.settings) { item in
                        IconButton(title: item.title, badge: item == .requests) { open(.setting(item)) } icon: {
                            SymbolTile(symbol: item.symbol, color: item.color, size: iconSize)
                        }
                    }
                }
            }
            .padding(.horizontal, 20)
            .padding(.bottom, 96)
            .frame(maxWidth: 1100, alignment: .leading)
            .frame(maxWidth: .infinity)
        }
        .groupedBackground()
        #if os(iOS)
        .navigationTitle("Settings")
        .navigationSubtitle(subtitle)
        .navigationBarTitleDisplayMode(.large)
        #endif
    }

    private var subtitle: String {
        let who = store.me.map { $0.admin ? String(localized: "\($0.name) · admin") : $0.name } ?? ""
        return [store.install?.name ?? "", who].filter { !$0.isEmpty }.joined(separator: " · ")
    }
}

extension Setting {
    /// Its icon's colour, as Settings colours its own.
    var color: Color {
        switch self {
        case .store: .blue
        case .groups: .green
        case .devices: .gray
        case .secrets: .indigo
        case .access: .purple
        case .users: .teal
        case .backups: .orange
        case .requests: .red
        }
    }
}
