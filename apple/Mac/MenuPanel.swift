import ServiceManagement
import SwiftUI

/// The app's own menu bar item's panel: this user's devices, which widgets have items, and a way into the app.
struct MenuPanel: View {
    @Environment(Store.self) private var store
    @Environment(\.openWindow) private var openWindow
    @State private var openAtLogin = SMAppService.mainApp.status == .enabled
    private var items: WidgetItems { .shared }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let install = store.install {
                HStack(spacing: 10) {
                    Image("Logo").resizable().frame(width: 26, height: 26)
                    Text(install.name).font(.headline)
                    Spacer()
                }
                .padding(.horizontal, 14)
                .padding(.top, 12)
                .padding(.bottom, 8)

                SectionTitle("Your Devices")
                ForEach(install.servers.filter { $0.user == store.me?.id }) { server in
                    HStack(spacing: 8) {
                        Circle()
                            .fill(server.isOnline ? AnyShapeStyle(.green) : AnyShapeStyle(.quaternary))
                            .frame(width: 7, height: 7)
                        Text(displayName(server.name))
                        Spacer()
                        if server.key == store.state?.machine.key {
                            Text("This device").foregroundStyle(.secondary)
                        } else if !server.isOnline {
                            Text("Offline").foregroundStyle(.tertiary)
                        }
                    }
                    .font(.callout)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 3)
                }
                if !store.allWidgets.isEmpty {
                    Divider().padding(.vertical, 6).padding(.horizontal, 14)
                    SectionTitle("In the Menu Bar")
                    ForEach(store.allWidgets) { widget in
                        MenuButton { items.toggle(widget.id) } label: {
                            HStack {
                                Label(widget.title ?? widget.widget, systemImage: symbol(store.app(widget.app)?.icon))
                                Spacer()
                                if !items.hidden.contains(widget.id) { Image(systemName: "checkmark").font(.callout.weight(.semibold)) }
                            }
                        }
                    }
                }
                Divider().padding(.vertical, 6).padding(.horizontal, 14)
            } else {
                Text(store.state == nil ? "Starting…" : "Not set up yet")
                    .foregroundStyle(.secondary)
                    .padding(14)
                Divider().padding(.horizontal, 14)
            }

            MenuButton(action: open) {
                Text("Open \(store.brand?.name ?? appName)")
            }
            MenuButton {
                openAtLogin.toggle()
                if openAtLogin { try? SMAppService.mainApp.register() } else { try? SMAppService.mainApp.unregister() }
            } label: {
                HStack {
                    Text("Open at Login")
                    Spacer()
                    if openAtLogin { Image(systemName: "checkmark").font(.callout.weight(.semibold)) }
                }
            }
            MenuButton { NSApp.terminate(nil) } label: {
                Text("Quit")
            }
            .padding(.bottom, 6)
        }
        .frame(width: 340)
    }

    private func open() {
        openWindow(id: "main")
        NSApp.activate()
    }
}

/// The app's own menu bar item: the brand's mark. It also lends the widget items a way to open the window.
struct MenuBarMark: View {
    var store: Store
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Image("MenuBarIcon")
            .accessibilityLabel(store.brand?.name ?? appName)
            .onAppear { WidgetItems.shared.openWindow = { openWindow(id: "main") } }
    }
}

private struct SectionTitle: View {
    var title: LocalizedStringKey
    init(_ title: LocalizedStringKey) { self.title = title }

    var body: some View {
        Text(title)
            .font(.caption.weight(.semibold))
            .foregroundStyle(.secondary)
            .padding(.horizontal, 14)
            .padding(.top, 4)
            .padding(.bottom, 4)
    }
}

/// A row that highlights under the pointer, as menu items do.
private struct MenuButton<Label: View>: View {
    var action: () -> Void
    @ViewBuilder var label: Label
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            label
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 8)
                .padding(.vertical, 5)
                .background(hovering ? AnyShapeStyle(.fill.tertiary) : AnyShapeStyle(.clear), in: .rect(cornerRadius: 6))
                .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .padding(.horizontal, 6)
    }
}
