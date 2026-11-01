import SwiftUI

/// Who opened this user's data, where and when.
struct AccessLogView: View {
    @Environment(Store.self) private var store

    var body: some View {
        let opens = (store.state?.opens ?? []).sorted { $0.time > $1.time }
        Form {
            Section {
                ForEach(opens) { open in
                    let server = store.install?.servers.first { $0.key == open.server }?.name ?? String(open.server.prefix(8))
                    HStack(spacing: 12) {
                        Avatar(id: open.by, name: store.install?.user(open.by)?.name ?? "", size: 32)
                        VStack(alignment: .leading, spacing: 2) {
                            Text("\(store.userName(open.by)) opened your data on \(displayName(server))\(open.agent.map { ", through \($0)" } ?? "")")
                            Text(Date(timeIntervalSince1970: open.time / 1000).formatted(date: .abbreviated, time: .shortened))
                                .font(.callout)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
                if opens.isEmpty { Text("Nobody has opened your data.").foregroundStyle(.secondary) }
            } footer: {
                Text("Whoever runs a server with your data can read it for an hour, and it shows here.")
            }
        }
        .formStyle(.grouped)
        .navigationTitle("Access Log")
    }
}

/// Everyone in the install, for admins: each one's role as a button on their row, with Remove there too, and
/// adding someone new. The last admin's role can't change.
struct UsersView: View {
    @Environment(Store.self) private var store
    @State private var newName = ""
    @State private var newRole = "member"
    @State private var inviting: (name: String, role: String)?
    @State private var error: String?
    /// Roles changed but not yet taken in, shown at once; nil for someone being removed.
    @State private var pending: [String: String?] = [:]

    var body: some View {
        let users = store.install?.users ?? []
        let admins = users.filter(\.admin).count
        Form {
            Section {
                ForEach(users.filter { pending[$0.id] != .some(nil) }) { user in
                    let role = pending[user.id].flatMap { $0 } ?? user.role
                    PersonRow(id: user.id, name: user.id == store.me?.id ? String(localized: "\(user.name) (you)") : user.name) {
                        if user.id == store.me?.id || (role == "admin" && admins == 1) {
                            Text(role == "admin" ? "Admin" : "Member").foregroundStyle(.secondary)
                        } else {
                            RoleMenu(role: role, removeTitle: "Remove \(user.name)") { change(user.id, to: $0) }
                        }
                    }
                }
            } header: {
                Text("People in \(store.install?.name ?? "")")
            } footer: {
                if let error {
                    Label(error, systemImage: "exclamationmark.circle.fill").foregroundStyle(.red)
                }
            }
            Section {
                TextField("Name", text: $newName)
                Picker("Role", selection: $newRole) {
                    Text("Member").tag("member")
                    Text("Admin").tag("admin")
                }
                Button("Show Their Pairing Code") { inviting = (newName.trimmingCharacters(in: .whitespaces), newRole) }
                    .disabled(newName.trimmingCharacters(in: .whitespaces).isEmpty)
            } header: {
                Text("Add Someone")
            } footer: {
                Text("They scan it on their own device.")
            }
        }
        .formStyle(.grouped)
        .navigationTitle("Users")
        .sheet(isPresented: Binding(get: { inviting != nil }, set: { if !$0 { inviting = nil } })) {
            PairSheet(newUser: inviting)
        }
    }

    /// Changes someone's role, or with none removes them: shown at once, and undone with why if it fails.
    private func change(_ user: String, to role: String?) {
        error = nil
        pending[user] = .some(role)
        Task {
            do {
                try await store.change { node in
                    if let role { try await node.setRole(user: user, role: role) } else { try await node.remove(user: user) }
                }
            } catch { self.error = error.localizedDescription }
            pending[user] = nil
        }
    }
}

/// The install's settings, for admins: entries like everything else, so every server and shell has the same ones.
struct InstallSettingsView: View {
    @Environment(Store.self) private var store
    @State private var madeHere = ""
    @State private var domain = ""
    @State private var error: String?
    /// The Vault item on Everyone switched on for push, by its title, and whether its sheet is open.
    @State private var pushKey: String?
    @State private var askingKey = false

    private var settings: [String: String] { store.install?.settings ?? [:] }
    /// The sidecar that sends pushes, empty for off: its Apple key is a Vault item switched on for its name.
    private var push: String { settings["push"] ?? "" }

    var body: some View {
        Form {
            Section {
                TextField("Apps Made Here", text: $madeHere, prompt: Text("Made here"))
                    .onSubmit { save("made_here", madeHere) }
            } header: {
                Text("App Store")
            }
            Section {
                Toggle("Push Notifications", isOn: Binding(get: { !push.isEmpty }, set: { set("push", $0 ? nil : "") }))
                if !push.isEmpty && store.isAdmin {
                    Button { askingKey = true } label: {
                        LabeledContent("Apple Push Key", value: pushKey ?? "")
                            .contentShape(.rect)
                    }
                    .buttonStyle(.plain)
                }
            }
            Section {
                TextField("Work Email Domain", text: $domain, prompt: Text("example.org"))
                    .onSubmit { save("join_domain", domain) }
            } header: {
                Text("Joining")
            } footer: {
                if let error {
                    Label(error, systemImage: "exclamationmark.circle.fill").foregroundStyle(.red)
                } else {
                    Text("People with an email at this domain join by signing in.")
                }
            }
        }
        .formStyle(.grouped)
        .navigationTitle("Install")
        .onAppear { madeHere = settings["made_here"] ?? ""; domain = settings["join_domain"] ?? "" }
        .task(id: push) { await loadKey() }
        .sheet(isPresented: $askingKey, onDismiss: { Task { await loadKey() } }) {
            VaultRequestSheet(scope: "global", id: "", ask: pushAsk, icon: "bell.badge.fill", color: "#ff3b30")
        }
        .onDisappear { save("made_here", madeHere); save("join_domain", domain) }
    }

    /// What push asks the Vault for: Apple's APNs key (its .p8 file), the team and key it's from, and the app's bundle
    /// id pushes go to, sent only to Apple's push servers.
    private var pushAsk: VaultRequest {
        let fields = [VaultRequest.Field(name: "key", secret: true, file: true), VaultRequest.Field(name: "team_id", secret: false),
                      VaultRequest.Field(name: "key_id", secret: false), VaultRequest.Field(name: "topic", secret: false)]
        return VaultRequest(id: "", scope: "global", scope_name: String(localized: "Everyone"), app: push, title: String(localized: "Apple Push Key"),
                            fields: fields, hosts: ["api.push.apple.com", "api.sandbox.push.apple.com"],
                            why: String(localized: "Phones get notifications through Apple with it."), by: store.me?.name ?? "",
                            time: Date.now.timeIntervalSince1970 * 1000, state: "waiting")
    }

    private func loadKey() async {
        guard !push.isEmpty else { return }
        pushKey = try? await store.node.vault(scope: "global").first { $0.apps.contains(push) }?.title
    }

    /// A text field's value, if it changed: empty puts back the edition's default.
    private func save(_ name: String, _ text: String) {
        let text = text.trimmingCharacters(in: .whitespaces)
        guard text != (settings[name] ?? "") else { return }
        set(name, text.isEmpty ? nil : text)
    }

    private func set(_ name: String, _ value: String?) {
        error = nil
        Task {
            do { try await store.change { try await $0.setSetting(name, value) } } catch { self.error = error.localizedDescription }
        }
    }
}

/// This server's encrypted backups to a bucket, for admins. The credentials stay on this server.
struct BackupsView: View {
    @Environment(Store.self) private var store
    @State private var url = ""
    @State private var accessKey = ""
    @State private var secretKey = ""
    @State private var key = ""
    @State private var status: String?
    @State private var error: String?

    var body: some View {
        Form {
            Section {
                TextField("Bucket", text: $url, prompt: Text("https://…r2.cloudflarestorage.com/backups"))
                TextField("Access Key", text: $accessKey)
                SecureField("Secret Key", text: $secretKey)
                SecureField("Backup Key", text: $key, prompt: Text("64 hex characters, from 1Password"))
            } header: {
                Text("Back Up \(displayName(store.state?.host?.name ?? store.state?.machine.name ?? ""))")
            } footer: {
                if let error {
                    Label(error, systemImage: "exclamationmark.circle.fill").foregroundStyle(.red)
                } else {
                    Text(status ?? "Encrypted with the backup key before it leaves.")
                }
            }
            Section {
                Button("Start Backing Up") { run(url) }
                    .disabled(url.isEmpty || accessKey.isEmpty || secretKey.isEmpty || key.isEmpty)
                Button("Stop Backing Up", role: .destructive) { run(nil) }
            }
        }
        .formStyle(.grouped)
        .navigationTitle("Backups")
    }

    private func run(_ url: String?) {
        error = nil
        Task {
            do {
                try await store.node.backup(url: url, accessKey: accessKey, secretKey: secretKey, key: key)
                status = url == nil ? String(localized: "Backups stopped.") : String(localized: "Backing up.")
                secretKey = ""; key = ""
            } catch { self.error = error.localizedDescription }
        }
    }
}

/// What waits for this user: items apps and agents asked them for, group invites, and servers offered to them to trust.
struct RequestsView: View {
    @Environment(Store.self) private var store
    @State private var error: String?
    @State private var asked: [VaultRequest] = []

    var body: some View {
        Form {
            if !asked.isEmpty {
                Section("Vault") {
                    ForEach(asked) { request in
                        HStack(spacing: 12) {
                            AppTile(app: store.app(request.app), seed: request.app, size: 32)
                            VStack(alignment: .leading, spacing: 2) {
                                Text("\(store.app(request.app)?.label ?? request.app) wants \(request.label)").font(.body.weight(.medium))
                                Text([request.asker, request.why].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · "))
                                    .font(.callout)
                                    .foregroundStyle(.secondary)
                                    .lineLimit(2)
                            }
                            Spacer()
                            Button("Choose…") { store.sheet = .vault(scope: request.scope, id: request.id) }
                                .buttonStyle(.borderedProminent)
                        }
                    }
                }
            }
            if !store.invites.isEmpty {
                Section("Invites") {
                    ForEach(store.invites) { group in
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(group.name).font(.body.weight(.medium))
                                Text(group.members.map { store.userName($0.user) }.formatted(.list(type: .and))).font(.callout).foregroundStyle(.secondary).lineLimit(1)
                            }
                            Spacer()
                            Button("Join") { run { try await $0.accept(group: group.id) } }
                                .buttonStyle(.borderedProminent)
                        }
                    }
                }
            }
            if !store.trustOffers.isEmpty {
                Section {
                    ForEach(store.trustOffers) { server in
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(displayName(server.name)).font(.body.weight(.medium))
                                Text("Lets it keep your own data").font(.callout).foregroundStyle(.secondary)
                            }
                            Spacer()
                            Button("Trust") { run { try await $0.trust(server: server.key, true) } }
                                .buttonStyle(.bordered)
                        }
                    }
                } header: {
                    Text("Servers")
                }
            }
            if asked.isEmpty && store.invites.isEmpty && store.trustOffers.isEmpty {
                Text("Nothing waits for you.").foregroundStyle(.secondary)
            }
            if let error {
                Label(error, systemImage: "exclamationmark.circle.fill").foregroundStyle(.red)
            }
        }
        .formStyle(.grouped)
        .navigationTitle("Requests")
        .task(id: store.state?.waitingRequests) {
            asked = (try? await store.node.vaultRequests())?.filter(\.waiting) ?? asked
        }
    }

    private func run(_ work: @escaping @MainActor (NodeClient) async throws -> Void) {
        error = nil
        Task { do { try await store.change(work) } catch { self.error = error.localizedDescription } }
    }
}
