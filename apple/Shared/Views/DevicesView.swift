import SwiftUI

/// This user's devices, every server they can see, which of them hold their data, and pairing another device.
struct DevicesView: View {
    @Environment(Store.self) private var store
    @State private var name = ""
    @State private var error: String?

    var body: some View {
        @Bindable var store = store
        let install = store.install, me = store.me?.id ?? ""
        let servers = install?.servers ?? [], clients = install?.clients ?? []
        let mine = servers.filter { $0.user == me }
        let others = servers.filter { $0.user != me }
        Form {
            Section {
                TextField("Name", text: $name)
                    .onSubmit(rename)
            } header: {
                Text("This Device")
            } footer: {
                Text("Others in \(install?.name ?? "the install") see it by this name.")
            }

            Section {
                ForEach(mine) { ServerRow(server: $0, here: $0.key == store.state?.machine.key) }
                ForEach(clients.filter { $0.user == me }) { ClientRow(client: $0, here: $0.key == store.state?.machine.key) }
                Button("Pair a Phone or Computer…", systemImage: "qrcode") { store.sheet = .pair }
            } header: {
                Text("Your Devices")
            } footer: {
                Text("A computer you pair runs the node and keeps your data; a phone keeps none and asks your servers.")
            }

            if !others.isEmpty {
                Section {
                    ForEach(others) { ServerRow(server: $0, here: false) }
                } header: {
                    Text("Servers")
                } footer: {
                    Text("Servers you may use. One you trust keeps your own data too; privileged servers keep everyone’s.")
                }
            }

            Section {
                let holding = servers.filter { holds($0, me) }
                Text(holding.map { displayName($0.name) }.formatted())
            } header: {
                Text("Where Your Data Lives")
            } footer: {
                if let error {
                    Label(error, systemImage: "exclamationmark.circle.fill").foregroundStyle(.red)
                } else {
                    Text("Your own data is kept only on these servers: yours, ones you trust, and privileged ones.")
                }
            }
        }
        .formStyle(.grouped)
        .navigationTitle("Devices")
        .onAppear { name = store.state?.machine.name ?? "" }
    }

    private func holds(_ server: Server, _ me: String) -> Bool {
        server.isPrivileged || server.owners.contains("user:\(me)") || server.trusted_by?.contains(me) == true
    }

    private func rename() {
        let new = name.trimmingCharacters(in: .whitespaces)
        guard !new.isEmpty, new != store.state?.machine.name else { return }
        Task {
            do { try await store.rename(new) } catch { self.error = error.localizedDescription }
        }
    }
}

private struct ServerRow: View {
    @Environment(Store.self) private var store
    var server: Server
    var here: Bool
    @State private var error: String?

    var body: some View {
        let me = store.me?.id ?? ""
        HStack(spacing: 14) {
            Image(systemName: deviceSymbol(server.name))
                .font(.system(size: 22, weight: .light))
                .foregroundStyle(server.isOnline ? AnyShapeStyle(.tint) : AnyShapeStyle(.tertiary))
                .frame(width: 34)
            VStack(alignment: .leading, spacing: 2) {
                Text(displayName(server.name)).font(.body.weight(.medium))
                Text(error ?? detail).font(.callout).foregroundStyle(error == nil ? AnyShapeStyle(.secondary) : AnyShapeStyle(.red))
            }
            Spacer()
            if mayTrust(me) {
                Toggle("Trust", isOn: Binding(get: { server.trusted_by?.contains(me) == true }, set: trust))
                    .labelsHidden()
                    .help("Trust it to keep your own data")
            } else {
                HStack(spacing: 6) {
                    Circle().fill(server.isOnline ? AnyShapeStyle(.green) : AnyShapeStyle(.quaternary)).frame(width: 8, height: 8)
                    Text(server.isOnline ? "Online" : "Offline").font(.callout).foregroundStyle(.secondary)
                }
            }
        }
        .padding(.vertical, 4)
        .contextMenu {
            if server.user == me && !here {
                Button("Remove \(displayName(server.name))", role: .destructive) {
                    Task { do { try await store.change { try await $0.remove(machine: server.key) } } catch { self.error = error.localizedDescription } }
                }
            }
        }
    }

    /// Whether this user may choose to trust it: they may use it, but don't own it, and it isn't privileged.
    private func mayTrust(_ me: String) -> Bool {
        !server.isPrivileged && !server.owners.contains("user:\(me)") && server.users.contains { p in
            p == "user:\(me)" || p == "install" || (p.hasPrefix("group:") && store.install?.group(String(p.dropFirst(6)))?.role(of: me) != nil)
        }
    }

    private var detail: String {
        if here { return String(localized: "This device") }
        if server.isPrivileged { return String(localized: "Privileged · keeps everyone’s data") }
        let owners = server.owners.map { owner -> String in
            if owner == "install" { return String(localized: "the install") }
            if owner.hasPrefix("group:") { return store.install?.group(String(owner.dropFirst(6)))?.name ?? owner }
            let id = String(owner.dropFirst(5))
            return id == store.me?.id ? String(localized: "you") : store.userName(id)
        }
        return String(localized: "Run by \(owners.formatted())")
    }

    private func trust(_ on: Bool) {
        Task {
            do { try await store.change { try await $0.trust(server: server.key, on) } } catch { self.error = error.localizedDescription }
        }
    }
}

private struct ClientRow: View {
    @Environment(Store.self) private var store
    var client: ClientInfo
    var here: Bool
    @State private var error: String?

    var body: some View {
        HStack(spacing: 14) {
            Image(systemName: deviceSymbol(client.name.isEmpty ? "phone" : client.name))
                .font(.system(size: 22, weight: .light))
                .foregroundStyle(.tint)
                .frame(width: 34)
            VStack(alignment: .leading, spacing: 2) {
                Text(displayName(client.name)).font(.body.weight(.medium))
                Text(error ?? (here ? String(localized: "This device") : asked)).font(.callout).foregroundStyle(.secondary)
            }
            Spacer()
        }
        .padding(.vertical, 4)
        .contextMenu {
            if !here {
                Button("Remove \(displayName(client.name))", role: .destructive) {
                    Task { do { try await store.change { try await $0.remove(machine: client.key) } } catch { self.error = error.localizedDescription } }
                }
            }
        }
    }

    private var asked: String {
        guard let asked = client.asked else { return String(localized: "Phone") }
        return String(localized: "Phone · asked \(Date(timeIntervalSince1970: asked / 1000).formatted(.relative(presentation: .named)))")
    }
}
