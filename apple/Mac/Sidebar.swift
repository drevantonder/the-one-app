import SwiftUI

/// The apps the space has screens for, then the shell's own views.
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
            SpaceHeader()
                .padding(.horizontal, 10)
                .padding(.bottom, 6)
        }
    }
}

/// The app's one space: another context is another app.
private struct SpaceHeader: View {
    @Environment(Store.self) private var store

    var body: some View {
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
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
    }
}
