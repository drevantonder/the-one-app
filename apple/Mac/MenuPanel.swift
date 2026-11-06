import ServiceManagement
import SwiftUI

/// The app's own menu bar item's panel: this user's devices, which widgets have items, and a way into the app.
struct MenuPanel: View {
    @Environment(Store.self) private var store
    @Environment(\.openWindow) private var openWindow
    @State private var openAtLogin = SMAppService.mainApp.status == .enabled
    private var items: WidgetItems { .shared }
    /// Whether agents may use this Mac, which Computer Use's helper app reads (apps/computer/service).
    @AppStorage("computerUse") private var letAgents = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let network = store.network, store.signedIn {
                HStack(spacing: 10) {
                    Image("Logo").resizable().frame(width: 26, height: 26)
                    Text(network.name).font(.headline)
                    Spacer()
                }
                .padding(.horizontal, 14)
                .padding(.top, 12)
                .padding(.bottom, 8)

                SectionTitle("Your Devices")
                ForEach(network.devices.filter { $0.user == store.me?.id }) { device in
                    HStack(spacing: 8) {
                        Circle()
                            .fill(device.isOnline ? Kit.text : Kit.faint)
                            .frame(width: 7, height: 7)
                        Text(displayName(device.name))
                        Spacer()
                        if device.key == store.device?.key {
                            Text("This device").foregroundStyle(.secondary)
                        } else if !device.isOnline {
                            Text("Offline").foregroundStyle(.tertiary)
                        }
                    }
                    .font(.callout)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 3)
                }
                if !store.allWidgets.isEmpty {
                    Divider().padding(.vertical, 6).padding(.horizontal, 14)
                    SectionTitle("Widgets")
                    ScrollView {
                        VStack(spacing: 8) {
                            ForEach(store.allWidgets) { widget in
                                PanelWidget(widget: widget, inMenuBar: !items.hidden.contains(widget.kind), toggle: { items.toggle(widget.kind) }, open: open)
                            }
                        }
                        .padding(.horizontal, 10)
                    }
                    .frame(maxHeight: 460)
                    .fixedSize(horizontal: false, vertical: true)
                }
                Divider().padding(.vertical, 6).padding(.horizontal, 14)
            } else {
                Group {
                    if store.state == nil {
                        Later(.milliseconds(400)) { Text("Starting…") }
                    } else {
                        Text(store.network == nil ? "Not set up yet" : "Signed out")
                    }
                }
                .foregroundStyle(.secondary)
                .padding(14)
                Divider().padding(.horizontal, 14)
            }

            MenuButton(action: open) {
                Text(store.signedIn || store.state == nil ? "Open \(store.config?.name ?? appName)" : "Sign In…")
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
            MenuButton {
                letAgents.toggle()
            } label: {
                HStack {
                    Text("Let Agents Use This Mac")
                    Spacer()
                    if letAgents { Image(systemName: "checkmark").font(.callout.weight(.semibold)) }
                }
            }
            .help("Your own agents may use this Mac's apps, keyboard and pointer. The menu bar shows when one does, with Stop.")
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

/// The app's own menu bar item: the config's mark. It also lends the widget items a way to open the window.
struct MenuBarMark: View {
    var store: Store
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Image("MenuBarIcon")
            .accessibilityLabel(store.config?.name ?? appName)
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

/// A widget in the panel, drawn live, with whether it also has an item of its own in the menu bar.
private struct PanelWidget: View {
    @Environment(Store.self) private var store
    var widget: WidgetInfo
    var inMenuBar: Bool
    var toggle: () -> Void
    var open: () -> Void

    var body: some View {
        let app = store.app(widget.app)
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                AppTile(app: app, seed: widget.app, size: 16)
                Text(widget.title ?? app?.label ?? widget.widget)
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                Spacer()
                Button(action: toggle) {
                    Image(systemName: inMenuBar ? "menubar.rectangle" : "menubar.dock.rectangle")
                        .foregroundStyle(inMenuBar ? AnyShapeStyle(.tint) : AnyShapeStyle(.tertiary))
                }
                .buttonStyle(.borderless)
                .help(inMenuBar ? "Shown in the menu bar: click to hide it there" : "Show in the menu bar")
            }
            if widget.error != nil {
                Label("This widget stopped working.", systemImage: "exclamationmark.triangle.fill")
                    .font(.callout)
                    .foregroundStyle(Kit.warn)
            } else {
                TimelineView(.everyMinute) { context in
                    WidgetTree(node: widget.tree("medium"), now: context.date)
                        .font(.callout)
                }
                .environment(\.widgetHost, WidgetHost(app: widget.app, scope: nil, run: { [store] op, input in await store.run(app: widget.app, op: op, input: input) },
                                                      open: { link in follow(link) }))
            }
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.fill.quaternary, in: .rect(cornerRadius: 10, style: .continuous))
        .contentShape(.rect)
        .onTapGesture { follow(widget.tree("medium").link) }
    }

    /// A web link in the browser; a screen, or the app's first, in the window.
    private func follow(_ link: WidgetLink?) {
        if case .url(let url) = link { NSWorkspace.shared.open(url); return }
        let page = store.pages.first { store.widgets(on: $0).contains { $0.kind == widget.kind } }
        if let link, let route = store.route(for: link, app: widget.app, scope: page?.scope) {
            store.selection = route
        } else if let app = store.app(widget.app), app.hasScreens {
            store.selection = store.open(app, from: page)
        } else {
            return
        }
        open()
    }
}
