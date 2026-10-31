import SwiftUI

struct DevicesView: View {
    @Environment(Store.self) private var store

    var body: some View {
        let space = store.space
        let devices = space?.sortedDevices ?? []
        let mine = devices.filter { $0.user == space?.user }
        let others = devices.filter { $0.user != space?.user }
        Form {
            Section {
                ForEach(mine) { DeviceRow(device: $0) }
            } header: {
                Text("Your Devices")
            } footer: {
                if others.isEmpty {
                    Text("Pair a device to bring it into \(space?.title ?? "this space"). It syncs everything here and can run its apps.")
                        .foregroundStyle(.secondary)
                }
            }
            if !others.isEmpty {
                Section("Others in \(space?.title ?? "This Space")") {
                    ForEach(others) { DeviceRow(device: $0) }
                }
            }
        }
        .formStyle(.grouped)
        .frame(maxWidth: 640)
        .frame(maxWidth: .infinity)
        .navigationTitle("Devices")
        .navigationSubtitle("\(devices.filter(\.online).count) of \(space?.nodes.count ?? 0) online")
    }
}

private struct DeviceRow: View {
    var device: Member

    var body: some View {
        HStack(spacing: 14) {
            Image(systemName: deviceSymbol(device.name))
                .font(.system(size: 22, weight: .light))
                .foregroundStyle(device.online || device.phone ? AnyShapeStyle(.tint) : AnyShapeStyle(.tertiary))
                .frame(width: 34)
            VStack(alignment: .leading, spacing: 2) {
                Text(displayName(device.name))
                    .font(.body.weight(.medium))
                Text(detail)
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            if device.phone {
                Text(device.phoneStatus)
                    .font(.callout)
                    .foregroundStyle(.secondary)
            } else {
                HStack(spacing: 6) {
                    Circle()
                        .fill(device.online ? AnyShapeStyle(.green) : AnyShapeStyle(.quaternary))
                        .frame(width: 8, height: 8)
                    Text(device.online ? "Online" : "Offline")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
            }
        }
        .padding(.vertical, 6)
    }

    private var detail: String {
        let role = device.role == "admin" ? String(localized: "Admin") : String(localized: "Member")
        return device.me ? String(localized: "This device · \(role)") : role
    }
}
