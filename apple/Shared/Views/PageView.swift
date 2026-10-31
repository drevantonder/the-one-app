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
        .background { PageBackground(seed: page.scope) }
        .refreshable { try? await store.refreshState(); await store.refreshApps() }
    }
}

/// A page's title, large at its top; on a Mac, the window's title and subtitle instead.
struct PageHeader: View {
    var title: String
    var subtitle: String

    var body: some View {
        #if os(macOS)
        Color.clear.frame(height: 4)
            .navigationTitle(title)
            .navigationSubtitle(subtitle)
        #else
        VStack(alignment: .leading, spacing: 2) {
            Text(title)
                .font(.largeTitle.bold())
            Text(subtitle)
                .font(.subheadline)
                .foregroundStyle(.secondary)
        }
        .padding(.top, 4)
        .padding(.horizontal, 4)
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

/// A widget on a translucent card, under its app's tile and title.
struct WidgetCard: View {
    var widget: WidgetInfo
    var app: AppInfo?

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 7) {
                AppTile(symbol: symbol(app?.icon), seed: widget.app, size: 18)
                Text((widget.title ?? app?.label ?? widget.widget).uppercased())
                    .font(.caption2.weight(.semibold))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            TimelineView(.everyMinute) { context in
                WidgetTree(node: widget.root, now: context.date)
                    .font(.subheadline)
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.regularMaterial, in: .rect(cornerRadius: 22, style: .continuous))
        .overlay { RoundedRectangle(cornerRadius: 22, style: .continuous).strokeBorder(.white.opacity(0.25), lineWidth: 0.5) }
        .contentShape(.rect(cornerRadius: 22))
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
                IconButton(title: app.label, symbol: symbol(app.icon), seed: app.name) { open(store.open(app, from: page)) }
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
struct IconButton: View {
    var title: String
    var symbol: String
    var seed: String
    var badge = false
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            VStack(spacing: 6) {
                AppTile(symbol: symbol, seed: seed, size: iconSize)
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
        .background(.regularMaterial, in: .rect(cornerRadius: 22, style: .continuous))
    }
}

/// The cog's page: the shell's own views, as icons.
struct SettingsPage: View {
    @Environment(Store.self) private var store
    var open: (Route) -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                PageHeader(title: String(localized: "Settings"), subtitle: subtitle)
                LazyVGrid(columns: [GridItem(.adaptive(minimum: iconSize + 18, maximum: iconSize + 34), spacing: 10, alignment: .top)],
                          alignment: .leading, spacing: 20) {
                    ForEach(store.settings) { item in
                        IconButton(title: item.title, symbol: item.symbol, seed: item.rawValue, badge: item == .requests) {
                            open(.setting(item))
                        }
                    }
                }
            }
            .padding(.horizontal, 20)
            .padding(.bottom, 96)
            .frame(maxWidth: 1100, alignment: .leading)
            .frame(maxWidth: .infinity)
        }
        .background { PageBackground(seed: "settings") }
    }

    private var subtitle: String {
        let who = store.me.map { $0.admin ? String(localized: "\($0.name) · admin") : $0.name } ?? ""
        return [store.install?.name ?? "", who].filter { !$0.isEmpty }.joined(separator: " · ")
    }
}
