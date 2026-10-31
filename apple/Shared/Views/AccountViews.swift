import SwiftUI

/// The secrets this user may set in a scope: names, apps and hosts, never values. They can be replaced or deleted.
struct SecretsView: View {
    @Environment(Store.self) private var store
    @State private var scope = "user"
    @State private var secrets: [SecretInfo]?
    @State private var error: String?
    @State private var editing: SecretInfo?
    @State private var adding = false

    var body: some View {
        let scopes = store.allPages.filter { store.canArrange($0.scope) }
        Form {
            Picker("Page", selection: $scope) {
                ForEach(scopes) { Text($0.title).tag($0.scope) }
            }
            Section {
                if let secrets {
                    ForEach(secrets) { secret in
                        Button { editing = secret } label: {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(secret.name).font(.body.monospaced()).foregroundStyle(.primary)
                                Text("\(store.app(secret.app)?.label ?? secret.app) · sent only to \(secret.hosts.formatted())")
                                    .font(.callout)
                                    .foregroundStyle(.secondary)
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .contentShape(.rect)
                        }
                        .buttonStyle(.plain)
                    }
                    .onDelete { rows in rows.map { secrets[$0] }.forEach(delete) }
                    if secrets.isEmpty { Text("No secrets here.").foregroundStyle(.secondary) }
                } else if error == nil {
                    ProgressView()
                }
            } footer: {
                if let error {
                    Label(error, systemImage: "exclamationmark.circle.fill").foregroundStyle(.red)
                } else {
                    Text("A secret is set, never read: nothing shows its value again, not even to you. Apps put it only into requests to its hosts.")
                }
            }
        }
        .formStyle(.grouped)
        .navigationTitle("Secrets")
        .toolbar {
            ToolbarItem(placement: .primaryAction) { Button("New Secret", systemImage: "plus") { adding = true } }
        }
        .task(id: scope) { await load() }
        .sheet(item: $editing) { SecretSheet(scope: scope, secret: $0) { await load() } }
        .sheet(isPresented: $adding) { SecretSheet(scope: scope, secret: nil) { await load() } }
    }

    private func load() async {
        error = nil
        do { secrets = try await store.node.secrets(scope: scope) } catch { secrets = nil; self.error = error.localizedDescription }
    }

    private func delete(_ secret: SecretInfo) {
        Task {
            do {
                try await store.node.setSecret(scope: scope, app: secret.app, name: secret.name, hosts: secret.hosts, value: nil)
                await load()
            } catch { self.error = error.localizedDescription }
        }
    }
}

/// Setting a new secret, or replacing one's value and hosts.
private struct SecretSheet: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    var scope: String
    var secret: SecretInfo?
    var saved: () async -> Void
    @State private var app = ""
    @State private var name = ""
    @State private var hosts = ""
    @State private var value = ""
    @State private var error: String?

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    if secret == nil {
                        Picker("App", selection: $app) {
                            ForEach(store.apps) { Text($0.label).tag($0.name) }
                        }
                        TextField("Name", text: $name, prompt: Text("gmail_token"))
                    }
                    TextField("Hosts", text: $hosts, prompt: Text("api.example.com, other.example.com"))
                    SecureField("Value", text: $value, prompt: Text(secret == nil ? "Value" : "New value"))
                } footer: {
                    if let error {
                        Label(error, systemImage: "exclamationmark.circle.fill").foregroundStyle(.red)
                    } else {
                        Text("The app puts {{\(secret?.name ?? (name.isEmpty ? "name" : name))}} where it goes; the node fills it in only on requests to these hosts.")
                    }
                }
                if let secret {
                    Section {
                        Button("Delete \(secret.name)", role: .destructive) { save(nil) }
                    }
                }
            }
            .formStyle(.grouped)
            .navigationTitle(secret?.name ?? String(localized: "New Secret"))
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { save(value) }.disabled(value.isEmpty || (secret == nil && (app.isEmpty || name.isEmpty)))
                }
            }
            .onAppear {
                app = secret?.app ?? store.apps.first?.name ?? ""
                hosts = secret?.hosts.joined(separator: ", ") ?? ""
            }
        }
        #if os(macOS)
        .frame(width: 420, height: 320)
        #endif
    }

    private func save(_ value: String?) {
        let hosts = hosts.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
        Task {
            do {
                try await store.node.setSecret(scope: scope, app: secret?.app ?? app, name: secret?.name ?? name, hosts: hosts, value: value)
                await saved()
                dismiss()
            } catch { self.error = error.localizedDescription }
        }
    }
}

/// Who opened this user's data, where and when.
struct AccessLogView: View {
    @Environment(Store.self) private var store

    var body: some View {
        let opens = (store.state?.opens ?? []).sorted { $0.time > $1.time }
        Form {
            Section {
                ForEach(opens) { open in
                    let server = store.install?.servers.first { $0.key == open.server }?.name ?? String(open.server.prefix(8))
                    VStack(alignment: .leading, spacing: 2) {
                        Text("\(store.userName(open.by)) opened your data on \(displayName(server))")
                        Text(Date(timeIntervalSince1970: open.time / 1000).formatted(date: .abbreviated, time: .shortened))
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    }
                }
                if opens.isEmpty { Text("Nobody has opened your data.").foregroundStyle(.secondary) }
            } footer: {
                Text("Only someone who runs a server that keeps your data can open it, for an hour, to read. Every time is shown here.")
            }
        }
        .formStyle(.grouped)
        .navigationTitle("Access Log")
    }
}

/// Everyone in the install, for admins: roles, removing, and adding someone new.
struct UsersView: View {
    @Environment(Store.self) private var store
    @State private var newName = ""
    @State private var newRole = "member"
    @State private var inviting: (name: String, role: String)?
    @State private var error: String?

    var body: some View {
        Form {
            Section {
                ForEach(store.install?.users ?? []) { user in
                    HStack {
                        Text(user.id == store.me?.id ? "\(user.name) (you)" : user.name)
                        Spacer()
                        Text(user.admin ? "Admin" : "Member").foregroundStyle(.secondary)
                    }
                    .contextMenu {
                        if user.id != store.me?.id {
                            Button(user.admin ? "Make Member" : "Make Admin") { run { try await $0.setRole(user: user.id, role: user.admin ? "member" : "admin") } }
                            Button("Remove \(user.name)", role: .destructive) { run { try await $0.remove(user: user.id) } }
                        }
                    }
                }
            } header: {
                Text("People in \(store.install?.name ?? "")")
            } footer: {
                if let error { Label(error, systemImage: "exclamationmark.circle.fill").foregroundStyle(.red) }
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
                Text("They scan it on their own device, which joins as them.")
            }
        }
        .formStyle(.grouped)
        .navigationTitle("Users")
        .sheet(isPresented: Binding(get: { inviting != nil }, set: { if !$0 { inviting = nil } })) {
            PairSheet(newUser: inviting)
        }
    }

    private func run(_ work: @escaping @MainActor (NodeClient) async throws -> Void) {
        error = nil
        Task { do { try await store.change(work) } catch { self.error = error.localizedDescription } }
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
                    Text(status ?? "Everything this server keeps goes to the bucket, encrypted with the backup key. The bucket sees only how many objects there are and their sizes.")
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

/// What waits for this user: group invites, and servers offered to them to trust.
struct RequestsView: View {
    @Environment(Store.self) private var store
    @State private var error: String?

    var body: some View {
        Form {
            if !store.invites.isEmpty {
                Section("Invites") {
                    ForEach(store.invites) { group in
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(group.name).font(.body.weight(.medium))
                                Text(group.members.count == 1 ? "1 member · it gets a page of its own" : "\(group.members.count) members · it gets a page of its own").font(.callout).foregroundStyle(.secondary)
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
                                Text("Offered to you; trusting it lets it keep your own data").font(.callout).foregroundStyle(.secondary)
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
            if store.invites.isEmpty && store.trustOffers.isEmpty {
                Text("Nothing waits for you.").foregroundStyle(.secondary)
            }
            if let error {
                Label(error, systemImage: "exclamationmark.circle.fill").foregroundStyle(.red)
            }
        }
        .formStyle(.grouped)
        .navigationTitle("Requests")
    }

    private func run(_ work: @escaping @MainActor (NodeClient) async throws -> Void) {
        error = nil
        Task { do { try await store.change(work) } catch { self.error = error.localizedDescription } }
    }
}
