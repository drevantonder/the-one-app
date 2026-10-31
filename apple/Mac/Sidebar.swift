import SwiftUI

/// One collapsible section per page, listing its apps; a section's name shows the page. Settings sit at the bottom.
/// Folded (⌘\), it shows only the apps' icons, each where it was, with a page's dot standing for its name.
struct Sidebar: View {
    @Environment(Store.self) private var store
    @AppStorage("collapsedPages") private var collapsed = ""

    var body: some View {
        if store.sidebarFolded { folded } else { full }
    }

    private var folded: some View {
        @Bindable var store = store
        return List(selection: $store.selection) {
            ForEach(store.pages) { page in
                Section {
                    ForEach(store.apps(on: page)) { app in
                        AppTile(app: app, seed: app.name, size: 24)
                            .frame(maxWidth: .infinity)
                            .help(app.label)
                            .accessibilityLabel(app.label)
                            .tag(store.open(app, from: page))
                    }
                } header: {
                    Button { store.selection = .page(page.scope) } label: {
                        Text(initials(page.title))
                            .font(.caption2.weight(.bold))
                            .foregroundStyle(store.selection == .page(page.scope) ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
                            .frame(maxWidth: .infinity)
                            .contentShape(.rect)
                    }
                    .buttonStyle(.plain)
                    .help(page.title)
                    .accessibilityLabel(page.title)
                }
            }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) { SettingsRow(folded: true) }
    }

    /// A page's initials, for its folded header: "You", "PT", "Ev".
    private func initials(_ title: String) -> String {
        let words = title.split(separator: " ")
        return words.count > 1 ? words.prefix(2).compactMap(\.first).map(String.init).joined().uppercased() : String(title.prefix(3))
    }

    private var full: some View {
        @Bindable var store = store
        return List(selection: $store.selection) {
            ForEach(store.pages) { page in
                Section(isExpanded: expanded(page)) {
                    ForEach(store.apps(on: page)) { app in
                        Label {
                            Text(app.label)
                        } icon: {
                            AppTile(app: app, seed: app.name, size: 20)
                        }
                        .badge(store.appBadge(app.name, on: page) ?? 0)
                        .tag(store.open(app, from: page))
                    }
                } header: {
                    Button { store.selection = .page(page.scope) } label: {
                        Text(page.title)
                            .foregroundStyle(store.selection == .page(page.scope) ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
                            // A long group name wraps rather than lose its last words.
                            .lineLimit(3)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .contentShape(.rect)
                    }
                    .buttonStyle(.plain)
                    .contextMenu {
                        if page.group != nil {
                            Button("Hide \(page.title)") { store.setHidden(page.scope, true) }
                        }
                        Button("Edit Pages…") { store.sheet = .pages }
                    }
                }
            }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) { SettingsRow() }
    }

    private func expanded(_ page: Page) -> Binding<Bool> {
        Binding {
            !collapsed.split(separator: " ").contains(Substring(page.scope))
        } set: { open in
            var set = Set(collapsed.split(separator: " ").map(String.init))
            if open { set.remove(page.scope) } else { set.insert(page.scope) }
            collapsed = set.sorted().joined(separator: " ")
        }
    }
}

/// The cog, kept at the sidebar's foot as a row of its own, counting unread notifications and waiting requests.
private struct SettingsRow: View {
    @Environment(Store.self) private var store
    var folded = false

    var body: some View {
        @Bindable var store = store
        List(selection: $store.selection) {
            Label {
                if !folded { Text("Settings") }
            } icon: {
                Image(systemName: "gearshape")
            }
            .badge(store.badge)
            .tag(Route.settings)
        }
        .scrollDisabled(true)
        .scrollContentBackground(.hidden)
        .frame(height: 44)
    }
}
