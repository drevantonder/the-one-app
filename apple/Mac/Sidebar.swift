import SwiftUI

/// One section per page, listing its apps; a section's name shows the page, and its apps stay listed. Then Settings',
/// its name showing Settings, listing its items.
/// Folded (⌘\), it shows only the icons, each where it was, with a section's initials standing for its name.
struct Sidebar: View {
    @Environment(Store.self) private var store

    var body: some View {
        if store.sidebarFolded { folded } else { full }
    }

    private var folded: some View {
        @Bindable var store = store
        return List(selection: $store.selection) {
            ForEach(store.pages) { page in
                Section {
                    ForEach(listed(on: page)) { app in
                        AppTile(app: app, seed: app.name, size: 24)
                            .frame(maxWidth: .infinity)
                            .help(app.label)
                            .accessibilityLabel(app.label)
                            .tag(store.open(app, from: page))
                    }
                    FolderTile(apps: store.services(on: page), size: 24)
                        .frame(maxWidth: .infinity)
                        .help("Services")
                        .accessibilityLabel("Services")
                        .tag(Route.services(page.scope))
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
            Section {
                ForEach(store.settingsItems) { item in
                    SettingIcon(item: item, size: 24)
                        .frame(maxWidth: .infinity)
                        .help(item.title)
                        .accessibilityLabel(item.title)
                        .tag(Route.setting(item.name))
                }
            } header: {
                Button { store.selection = .settings } label: {
                    Text(initials(String(localized: "Settings")))
                        .font(.caption2.weight(.bold))
                        .foregroundStyle(store.selection == .settings ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
                        .frame(maxWidth: .infinity)
                        .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .help("Settings")
                .accessibilityLabel("Settings")
            }
        }
    }

    /// A page's initials, for its folded header: "You", "PT", "Ev".
    /// A page's apps, but those Settings lists (the App Store), which the sidebar shows once, under Settings.
    private func listed(on page: Page) -> [AppInfo] {
        store.apps(on: page).filter { app in !store.settingsItems.contains { $0.app == app.name } }
    }

    private func initials(_ title: String) -> String {
        let words = title.split(separator: " ")
        return words.count > 1 ? words.prefix(2).compactMap(\.first).map(String.init).joined().uppercased() : String(title.prefix(3))
    }

    private var full: some View {
        @Bindable var store = store
        return List(selection: $store.selection) {
            ForEach(store.pages) { page in
                // Held open: macOS folds a sidebar's section when its header is pressed (through accessibility, as
                // agents drive the app), and here the header opens the page.
                Section(isExpanded: Binding(get: { true }, set: { _ in })) {
                    ForEach(listed(on: page)) { app in
                        Label {
                            Text(app.label)
                        } icon: {
                            AppTile(app: app, seed: app.name, size: 20)
                        }
                        .badge(store.appBadge(app.name, on: page) ?? 0)
                        .tag(store.open(app, from: page))
                    }
                    // Its services, in one row, as the page has them in one folder, there even with none.
                    Label {
                        Text("Services")
                    } icon: {
                        FolderTile(apps: store.services(on: page), size: 20)
                    }
                    .tag(Route.services(page.scope))
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
                }
                .collapsible(false)
            }
            Section(isExpanded: Binding(get: { true }, set: { _ in })) {
                ForEach(store.settingsItems) { item in
                    Label {
                        // One that opens in the browser (Passkeys) says so, as macOS marks a link that leaves the app.
                        HStack {
                            Text(item.title)
                            if item.url != nil {
                                Spacer()
                                Image(systemName: "arrow.up.right")
                                    .font(.caption.weight(.semibold))
                                    .foregroundStyle(.tertiary)
                                    .accessibilityHidden(true)
                            }
                        }
                    } icon: {
                        SettingIcon(item: item, size: 20)
                    }
                    .badge(item.badge ?? 0)
                    .tag(Route.setting(item.name))
                }
            } header: {
                Button { store.selection = .settings } label: {
                    Text("Settings")
                        .foregroundStyle(store.selection == .settings ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .contentShape(.rect)
                }
                .buttonStyle(.plain)
            }
            .collapsible(false)
        }
    }
}

/// A page's Services folder, as an iOS folder's icon: its apps' icons, small, nine to a page, on frosted glass;
/// with none, nine faint empty places, so it reads as an empty folder.
private struct FolderTile: View {
    var apps: [AppInfo]
    var size: CGFloat

    var body: some View {
        let inset = size * 0.13, gap = size * 0.06
        let mini = (size - inset * 2 - gap * 2) / 3
        let shape = RoundedRectangle(cornerRadius: size * 0.225, style: .continuous)
        LazyVGrid(columns: Array(repeating: GridItem(.fixed(mini), spacing: gap), count: 3), alignment: .leading, spacing: gap) {
            if apps.isEmpty {
                ForEach(0..<9, id: \.self) { _ in
                    RoundedRectangle(cornerRadius: mini * 0.25, style: .continuous)
                        .fill(Color.gray.opacity(0.18))
                        .frame(width: mini, height: mini)
                }
            } else {
                ForEach(apps.prefix(9)) { app in
                    AppTile(app: app, seed: app.name, size: mini)
                }
            }
        }
        .padding(inset)
        .frame(width: size, height: size, alignment: .topLeading)
        .background(Color.gray.opacity(0.22), in: shape)
        .overlay(shape.strokeBorder(Color.gray.opacity(0.3), lineWidth: 0.5))
        .accessibilityHidden(true)
    }
}

/// A Settings item's icon: its symbol on its colour, as an app's tile.
private struct SettingIcon: View {
    var item: SettingsItem
    var size: CGFloat

    var body: some View {
        AppTile(icon: item.icon, seed: item.name, color: item.color, size: size)
    }
}
