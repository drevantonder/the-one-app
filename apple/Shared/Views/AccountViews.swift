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
                } else {
                    Text("Admins keep the install’s recovery key. It opens every backup, everyone’s.")
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
    /// The Vault item on Everyone switched on for push, by its title.
    @State private var pushKey: String?

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
                    Button { store.sheet = .ask(pushAsk) } label: {
                        HStack {
                            LabeledContent("Apple Push Key", value: pushKey ?? "")
                            Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
                        }
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
        .onChange(of: store.sheet) { if store.sheet == nil { Task { await loadKey() } } }
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

    /// A text field's value, if it changed: empty puts back the white label's default.
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

/// Backups, for admins: how they're going first, in words; then each place they go to, and the snapshots.
struct BackupsView: View {
    @Environment(Store.self) private var store
    @State private var backups: Backups?
    @State private var adding = false
    @State private var error: String?
    @State private var asked: Date?

    var body: some View {
        let places = backups?.places ?? []
        Form {
            Section {
                status(places)
            } footer: {
                if let error { Label(error, systemImage: "exclamationmark.circle.fill").foregroundStyle(.red) }
            }
            Section {
                ForEach(places) { place in
                    NavigationLink {
                        PlaceView(place: place) { await load() }
                    } label: {
                        PlaceRow(place: place)
                    }
                }
                Button("Add Place…") { adding = true }
            } header: {
                Text("Places")
            } footer: {
                Text("Encrypted before it leaves. The install’s recovery key opens every backup, everyone’s.")
            }
            if let snapshots = backups?.snapshots, !snapshots.isEmpty {
                Section("Snapshots") {
                    ForEach(snapshots.prefix(8), id: \.self) { s in
                        NavigationLink(when(s.time, style: .day)) { SnapshotView(snapshot: s) }
                    }
                }
            }
        }
        .formStyle(.grouped)
        .navigationTitle("Backups")
        .task { await load() }
        .task {
            // Times like "12 min ago" stay true.
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(30))
                await load()
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: .nodeEvent)) { _ in Task { await load() } }
        .sheet(isPresented: $adding, onDismiss: { Task { await load() } }) {
            AddPlaceSheet(disks: backups?.disks ?? [], here: backups?.here ?? "")
        }
    }

    /// Status first, in words: backed up when; or what's wrong, since when, and its fix.
    @ViewBuilder private func status(_ places: [Backups.Place]) -> some View {
        let failing = places.filter { $0.problem != nil }.min { $0.problem!.since < $1.problem!.since }
        let problem = failing?.problem
        let newest = places.compactMap(\.last).max { $0.time < $1.time }
        let color: Color = problem != nil ? .red : newest == nil ? .secondary : .green
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .top, spacing: 12) {
                Image(systemName: problem != nil ? "exclamationmark" : newest == nil ? "externaldrive" : "checkmark")
                    .font(.system(size: 18, weight: .bold))
                    .foregroundStyle(color)
                    .frame(width: 40, height: 40)
                    .background(Circle().fill(color.opacity(0.15)))
                VStack(alignment: .leading, spacing: 3) {
                    if let problem {
                        let title = failing?.title ?? ""
                        Text(failing?.last.map { "Not backed up to \(title) since \(when($0.time, style: .relative))" } ?? "Not backed up to \(title) yet").font(.title3.weight(.semibold))
                        Text(problem.text).foregroundStyle(.secondary)
                        Text(problem.fix).foregroundStyle(.secondary)
                    } else if let newest {
                        Text("Backed up \(when(newest.time, style: .relative))").font(.title3.weight(.semibold))
                    } else if places.isEmpty {
                        Text("Not backed up").font(.title3.weight(.semibold))
                        Text("Add a disk, or S3-compatible storage like Cloudflare R2, and your servers back up to it every hour that something changes.")
                            .foregroundStyle(.secondary)
                    } else {
                        Text("Backing up…").font(.title3.weight(.semibold))
                    }
                }
            }
            if !places.isEmpty {
                Button(problem == nil ? "Back Up Now" : "Try Again") { backUp() }
                    .buttonStyle(.bordered)
                    .disabled(asked.map { Date.now.timeIntervalSince($0) < 10 } ?? false)
            }
        }
        .padding(.vertical, 4)
    }

    private func load() async {
        do { backups = try await store.node.backups() } catch { self.error = error.localizedDescription }
    }

    private func backUp() {
        asked = .now
        Task {
            do { try await store.node.backUpNow() } catch { self.error = error.localizedDescription }
        }
    }
}

private enum WhenStyle { case relative, time, day }

/// A time for people: "12 min ago", "14:00", or "Today, 14:00".
private func when(_ ms: Double, style: WhenStyle) -> String {
    let date = Date(timeIntervalSince1970: ms / 1000)
    switch style {
    case .relative:
        return Date.now.timeIntervalSince(date) < 60 ? String(localized: "just now") : date.formatted(.relative(presentation: .named, unitsStyle: .abbreviated))
    case .time:
        return Calendar.current.isDateInToday(date) ? date.formatted(date: .omitted, time: .shortened) : date.formatted(date: .abbreviated, time: .shortened)
    case .day:
        let time = date.formatted(date: .omitted, time: .shortened)
        if Calendar.current.isDateInToday(date) { return String(localized: "Today, \(time)") }
        if Calendar.current.isDateInYesterday(date) { return String(localized: "Yesterday, \(time)") }
        return date.formatted(.dateTime.weekday(.wide).day().month().hour().minute())
    }
}

/// A place backups go to: what it is, and when it last backed up, or what's wrong.
private struct PlaceRow: View {
    var place: Backups.Place

    var body: some View {
        HStack(spacing: 12) {
            PlaceIcon(kind: place.kind)
            VStack(alignment: .leading, spacing: 2) {
                Text(place.title)
                if let problem = place.problem {
                    Text(problem.text).font(.callout).foregroundStyle(.red).lineLimit(2)
                } else {
                    Text(place.kind == "folder" ? String(localized: "Disk on \(displayName(place.server ?? ""))") : String(localized: "Off-site · \(place.where)"))
                        .font(.callout).foregroundStyle(.secondary).lineLimit(1)
                }
            }
            Spacer()
            if let last = place.last, place.problem == nil {
                Text(when(last.time, style: .time)).foregroundStyle(.secondary).monospacedDigit()
            }
        }
    }
}

private struct PlaceIcon: View {
    var kind: String

    var body: some View {
        Image(systemName: kind == "folder" ? "externaldrive.fill" : "cloud.fill")
            .font(.system(size: 15))
            .foregroundStyle(.white)
            .frame(width: 30, height: 30)
            .background(RoundedRectangle(cornerRadius: 7).fill(kind == "folder" ? Color.gray : Color.orange))
    }
}

/// A place's own page: what's wrong and its fix, where it is, its last backup, its item in the Vault, and stopping.
private struct PlaceView: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    var place: Backups.Place
    var changed: () async -> Void
    @State private var item: VaultItem?
    @State private var removing = false
    @State private var error: String?

    var body: some View {
        Form {
            if let problem = place.problem {
                Section {
                    Label(problem.text, systemImage: "exclamationmark.circle.fill").foregroundStyle(.red)
                    Text(problem.fix)
                }
            }
            Section {
                if place.kind == "folder" {
                    LabeledContent("Folder") { Text(place.where).textSelection(.enabled) }
                    LabeledContent("Disk on", value: displayName(place.server ?? ""))
                } else {
                    LabeledContent("Bucket", value: place.where)
                    if let endpoint = place.endpoint {
                        LabeledContent("Endpoint") { Text(endpoint).textSelection(.enabled).lineLimit(2) }
                    }
                }
                if let last = place.last {
                    LabeledContent("Last backup", value: "\(when(last.time, style: .day)), \(displayName(last.server))")
                }
            }
            if let item {
                Section {
                    NavigationLink("Edit in Vault") { VaultItemView(item: item) { await changed() } }
                }
            }
            Section {
                Button("Stop Backing Up Here", role: .destructive) { removing = true }
            } footer: {
                if let error { Label(error, systemImage: "exclamationmark.circle.fill").foregroundStyle(.red) }
            }
        }
        .formStyle(.grouped)
        .navigationTitle(place.title)
        .task { item = try? await store.node.vault(scope: "global").first { $0.id == place.id } }
        .confirmationDialog("Stop backing up to \(place.title)?", isPresented: $removing) {
            Button("Stop Backing Up", role: .destructive) { remove() }
        } message: {
            Text("What’s there now stays there.")
        }
    }

    private func remove() {
        Task {
            do {
                try await store.node.removePlace(place.id)
                await changed()
                dismiss()
            } catch { self.error = error.localizedDescription }
        }
    }
}

/// A snapshot of the install: when, where it went, whose data it holds, and which servers wrote it.
private struct SnapshotView: View {
    var snapshot: Backups.Snapshot

    var body: some View {
        Form {
            LabeledContent("Taken", value: Date(timeIntervalSince1970: snapshot.time / 1000).formatted(date: .complete, time: .shortened))
            LabeledContent("Kept in", value: snapshot.places.formatted())
            LabeledContent("Holds", value: snapshot.scopes.formatted())
            LabeledContent("Written by", value: snapshot.servers.map(displayName).formatted())
        }
        .formStyle(.grouped)
        .navigationTitle(when(snapshot.time, style: .day))
    }
}

/// Adds a place: S3-compatible storage by its endpoint, bucket and key, or a disk plugged into the server. Each is
/// tried (a test file written, read back and deleted) before it's added, so a typo shows here, not in a failed backup.
struct AddPlaceSheet: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    var disks: [Backups.Disk]
    var here: String
    @State private var cloud = true
    @State private var endpoint = ""
    @State private var bucket = ""
    @State private var keyID = ""
    @State private var secret = ""
    @State private var title = ""
    @State private var folder = ""
    @State private var tried: [Backups.Step] = []
    @State private var added = false
    @State private var busy = false
    @State private var error: String?

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Picker("Where", selection: $cloud) {
                        Text("Cloud Storage").tag(true)
                        Text("Disk").tag(false)
                    }
                    .pickerStyle(.segmented)
                    .labelsHidden()
                    .disabled(added)
                }
                if cloud {
                    Section {
                        TextField("Endpoint", text: $endpoint, prompt: Text("https://….r2.cloudflarestorage.com"))
                            #if os(iOS)
                            .keyboardType(.URL).textInputAutocapitalization(.never)
                            #endif
                            .autocorrectionDisabled()
                        TextField("Bucket", text: $bucket, prompt: Text("one-backups"))
                            #if os(iOS)
                            .textInputAutocapitalization(.never)
                            #endif
                            .autocorrectionDisabled()
                        TextField("Access Key ID", text: $keyID)
                            #if os(iOS)
                            .textInputAutocapitalization(.never)
                            #endif
                            .autocorrectionDisabled()
                        SecureField("Secret Key", text: $secret)
                        TextField("Name", text: $title, prompt: Text("Cloudflare R2"))
                    } header: {
                        Text("S3-Compatible Storage")
                    } footer: {
                        Text("Cloudflare R2, Backblaze B2, MinIO or any other S3 storage. Use a key that can reach only this bucket. It goes into the Vault and never leaves your servers.")
                    }
                } else {
                    Section {
                        ForEach(disks) { disk in
                            Button { folder = disk.path; title = disk.name } label: {
                                HStack(spacing: 12) {
                                    Image(systemName: "externaldrive.fill").foregroundStyle(.secondary).frame(width: 30)
                                    VStack(alignment: .leading, spacing: 2) {
                                        Text(disk.name).foregroundStyle(.primary)
                                        Text("Plugged into \(displayName(here)) · \(Int64(disk.free).formatted(.byteCount(style: .file))) free").font(.callout).foregroundStyle(.secondary)
                                    }
                                    Spacer()
                                    if folder == disk.path { Image(systemName: "checkmark").foregroundStyle(.tint) }
                                }
                                .contentShape(.rect)
                            }
                            .buttonStyle(.plain)
                        }
                        TextField("Folder", text: $folder, prompt: Text("/Volumes/…"))
                            .autocorrectionDisabled()
                    } header: {
                        Text("A Disk on \(displayName(here))")
                    } footer: {
                        Text("A disk keeps a copy at home too. It’s backed up while it’s plugged in.")
                    }
                }
                if !tried.isEmpty {
                    Section {
                        ForEach(tried) { step in
                            HStack {
                                Text(step.step)
                                Spacer()
                                Image(systemName: step.error == nil ? "checkmark" : "xmark").foregroundStyle(step.error == nil ? .green : .red)
                            }
                            if let error = step.error {
                                Text(error).font(.callout).foregroundStyle(.red)
                            }
                        }
                    } header: {
                        Text("Tried It")
                    } footer: {
                        if added { Text("The first backup has started. After that, every hour that something changes.") }
                    }
                }
                if let error {
                    Label(error, systemImage: "exclamationmark.circle.fill").foregroundStyle(.red)
                }
            }
            .formStyle(.grouped)
            .navigationTitle("Add Place")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                if added {
                    ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
                } else {
                    ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Add") { add() }.disabled(!ready || busy)
                    }
                }
            }
        }
        #if os(macOS)
        .frame(minWidth: 460, minHeight: 480)
        #endif
    }

    private var ready: Bool {
        let filled = { (s: String) in !s.trimmingCharacters(in: .whitespaces).isEmpty }
        return cloud ? [endpoint, bucket, keyID, secret].allSatisfy(filled) : filled(folder)
    }

    private func add() {
        busy = true
        error = nil
        var fields = cloud ? ["endpoint": endpoint, "bucket": bucket, "key_id": keyID, "secret": secret] : ["path": folder]
        if !title.trimmingCharacters(in: .whitespaces).isEmpty { fields["title"] = title }
        Task {
            do {
                let reply = try await store.node.addPlace(fields)
                tried = reply.tried
                added = reply.added
                if added { secret = "" }
            } catch { self.error = error.localizedDescription }
            busy = false
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
                                Text("\(store.label(request.app)) wants \(request.label)").font(.body.weight(.medium))
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
