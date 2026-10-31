import SwiftUI

/// The apps this space has screens for, then the shell's own views.
struct Sidebar: View {
    @Environment(Store.self) private var store

    var body: some View {
        @Bindable var store = store
        List(selection: $store.selection) {
            Section {
                ForEach(store.screenApps) { app in
                    Label(app.label, systemImage: symbol(app.icon)).tag(store.main(app))
                }
                Label("Devices", systemImage: "laptopcomputer.and.iphone")
                    .badge(store.space.map { "\($0.devices.filter(\.online).count)/\($0.devices.count)" } ?? "")
                    .tag(Route.devices)
            }
        }
        .safeAreaInset(edge: .top, spacing: 0) {
            SpaceSwitcher()
                .padding(.horizontal, 10)
                .padding(.bottom, 6)
        }
    }
}

private struct SpaceSwitcher: View {
    @Environment(Store.self) private var store

    var body: some View {
        @Bindable var store = store
        Menu {
            Picker("Space", selection: $store.spaceID) {
                ForEach(store.spaces) { space in
                    Text(space.title).tag(Optional(space.id))
                }
            }
            .pickerStyle(.inline)
            .labelsHidden()
            Divider()
            Button("New Space…", systemImage: "plus") { store.sheet = .newSpace }
            Button("Join with a Pairing Code…", systemImage: "qrcode.viewfinder") { store.sheet = .join }
        } label: {
            HStack(spacing: 10) {
                SpaceMark(name: store.space?.title ?? "", size: 30)
                VStack(alignment: .leading, spacing: 1) {
                    Text(store.space?.title ?? "")
                        .font(.headline)
                        .lineLimit(1)
                    if let space = store.space {
                        Text(space.devices.count == 1 ? "1 device" : "\(space.devices.count) devices")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                Spacer(minLength: 4)
                Image(systemName: "chevron.up.chevron.down")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
            .contentShape(.rect(cornerRadius: 10))
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
    }
}
