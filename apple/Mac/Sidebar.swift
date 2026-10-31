import SwiftUI

/// One collapsible section per page, listing its apps; a section's name shows the page. Settings sit at the bottom.
struct Sidebar: View {
    @Environment(Store.self) private var store
    @AppStorage("collapsedPages") private var collapsed = ""

    var body: some View {
        @Bindable var store = store
        List(selection: $store.selection) {
            ForEach(store.pages) { page in
                Section(isExpanded: expanded(page)) {
                    ForEach(store.apps(on: page)) { app in
                        Label {
                            Text(app.label)
                        } icon: {
                            AppTile(symbol: symbol(app.icon), seed: app.name, size: 20)
                        }
                        .tag(store.open(app, from: page))
                    }
                } header: {
                    Button { store.selection = .page(page.scope) } label: {
                        Text(page.title.uppercased())
                            .foregroundStyle(store.selection == .page(page.scope) ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
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
        .safeAreaInset(edge: .bottom, spacing: 0) {
            SettingsButton()
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
        }
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

/// The cog, with a red dot while a request waits.
private struct SettingsButton: View {
    @Environment(Store.self) private var store
    @State private var hovering = false

    var body: some View {
        let selected = store.selection == .settings
        Button { store.selection = .settings } label: {
            HStack(spacing: 8) {
                Image(systemName: "gearshape")
                    .font(.system(size: 14, weight: .medium))
                    .overlay(alignment: .topTrailing) {
                        if store.hasRequests { Circle().fill(.red).frame(width: 7, height: 7).offset(x: 3, y: -2) }
                    }
                Text("Settings")
                Spacer()
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
            .background(selected ? AnyShapeStyle(.tint.opacity(0.18)) : hovering ? AnyShapeStyle(.fill.tertiary) : AnyShapeStyle(.clear),
                        in: .rect(cornerRadius: 8))
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
    }
}
