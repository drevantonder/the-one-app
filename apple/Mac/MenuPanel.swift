import ServiceManagement
import SwiftUI

struct MenuLabel: View {
    var store: Store

    var body: some View {
        let text = store.recent.first?.text ?? store.brand?.name ?? appName
        let line = text.count > 44 ? text.prefix(42).trimmingCharacters(in: .whitespaces) + "…" : text
        Text("\(Image(systemName: "text.bubble"))  \(line)")
    }
}

/// The menu bar extra's panel: the latest posts, this space's devices, and a way into the app.
struct MenuPanel: View {
    @Environment(Store.self) private var store
    @Environment(\.openWindow) private var openWindow
    @State private var openAtLogin = SMAppService.mainApp.status == .enabled

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let space = store.space {
                HStack(spacing: 10) {
                    SpaceMark(name: space.title, size: 26)
                    Text(space.title).font(.headline)
                    Spacer()
                }
                .padding(.horizontal, 14)
                .padding(.top, 12)
                .padding(.bottom, 8)

                SectionTitle("Latest")
                if store.recent.isEmpty {
                    Text("Nothing on the board yet")
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 14)
                        .padding(.vertical, 6)
                }
                ForEach(store.recent.prefix(4)) { post in
                    MenuButton {
                        store.selection = .board(post.topic)
                        open()
                    } label: {
                        PostRow(post: post)
                    }
                }

                Divider().padding(.vertical, 6).padding(.horizontal, 14)
                SectionTitle("Devices")
                ForEach(space.sortedDevices) { device in
                    HStack(spacing: 8) {
                        Circle()
                            .fill(device.online ? AnyShapeStyle(.green) : AnyShapeStyle(.quaternary))
                            .frame(width: 7, height: 7)
                        Text(displayName(device.name))
                        Spacer()
                        if device.me {
                            Text("This device").foregroundStyle(.secondary)
                        } else if !device.online {
                            Text("Offline").foregroundStyle(.tertiary)
                        }
                    }
                    .font(.callout)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 3)
                }
                Divider().padding(.vertical, 6).padding(.horizontal, 14)
            } else {
                Text(store.state == nil ? "Starting…" : "No space yet")
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

private struct PostRow: View {
    var post: Post

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Avatar(name: post.name, person: post.person, size: 24)
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(displayName(post.name)).fontWeight(.semibold)
                    if !post.topic.isEmpty {
                        Text(post.topic).foregroundStyle(.secondary).lineLimit(1)
                    }
                    Spacer(minLength: 4)
                    Text(post.date.relative).foregroundStyle(.secondary)
                }
                .font(.caption)
                Text(post.text)
                    .font(.callout)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
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
