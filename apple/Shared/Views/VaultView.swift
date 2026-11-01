import SwiftUI

/// The Vault: the items on each page this user keeps (theirs, their groups' as an admin, everyone's as an admin),
/// searchable, each with what it's for and the apps that use it.
struct VaultView: View {
    @Environment(Store.self) private var store
    @State private var scope = "user"
    @State private var items: [VaultItem]?
    @State private var search = ""
    @State private var error: String?
    @State private var adding = false
    @State private var deleting: VaultItem?

    var body: some View {
        let pages = store.allPages.filter { store.canArrange($0.scope) }
        List {
            if pages.count > 1 {
                Picker("Page", selection: $scope) {
                    ForEach(pages) { Text($0.title).tag($0.scope) }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .listRowBackground(Color.clear)
                .listRowInsets(EdgeInsets())
            }
            if let items {
                let shown = items.filter(matches)
                Section {
                    ForEach(shown) { item in
                        NavigationLink {
                            VaultItemView(item: item) { await load() }
                        } label: {
                            VaultRow(item: item)
                        }
                        .swipeActions(edge: .trailing) {
                            Button(item.isAccount ? "Sign Out" : "Delete", role: .destructive) { deleting = item }
                        }
                        .contextMenu {
                            Button(item.isAccount ? "Sign Out" : "Delete", systemImage: "trash", role: .destructive) { deleting = item }
                        }
                    }
                    if shown.isEmpty {
                        Text(search.isEmpty ? "Nothing here yet." : "No matches.").foregroundStyle(.secondary)
                    }
                }
            } else if error == nil {
                ProgressView().frame(maxWidth: .infinity)
            }
            if let error {
                Label(error, systemImage: "exclamationmark.circle.fill").foregroundStyle(.red)
            }
        }
        #if os(iOS)
        .listStyle(.insetGrouped)
        #endif
        .searchable(text: $search)
        .navigationTitle("Vault")
        .toolbar {
            ToolbarItem(placement: .primaryAction) { Button("New Item", systemImage: "plus") { adding = true } }
        }
        .task(id: scope) { await load() }
        .refreshable { await load() }
        .onReceive(NotificationCenter.default.publisher(for: .nodeEvent)) { note in
            // The node's own entries (an item changed) change no app.
            guard let json = note.object as? String, let event = try? JSONDecoder().decode(NodeEvent.self, from: Data(json.utf8)),
                  event.type == "changed", event.apps?.isEmpty ?? true else { return }
            Task { await load() }
        }
        .sheet(isPresented: $adding) { NewItemSheet(scope: scope) { await load() } }
        .confirmationDialog(deleting.map { $0.isAccount ? "Sign out of \($0.title)?" : "Delete \($0.title)?" } ?? "",
                            isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }), titleVisibility: .visible,
                            presenting: deleting) { item in
            Button(item.isAccount ? "Sign Out" : "Delete", role: .destructive) { Task { await delete(item) } }
        } message: { item in
            Text(item.apps.isEmpty ? "It can’t be undone." : "\(appNames(item.apps, store: store)) stop using it.")
        }
    }

    private func matches(_ item: VaultItem) -> Bool {
        let q = search.trimmingCharacters(in: .whitespaces)
        guard !q.isEmpty else { return true }
        let words = [item.title, item.type ?? "", item.domain ?? ""] + item.hosts + item.apps.map { store.app($0)?.label ?? $0 }
            + item.fields.compactMap { $0.secret ? nil : $0.value }
        return words.contains { $0.localizedCaseInsensitiveContains(q) }
    }

    private func load() async {
        do {
            items = try await store.node.vault(scope: scope)
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func delete(_ item: VaultItem) async {
        do {
            try await store.delete(item)
        } catch {
            self.error = error.localizedDescription
        }
        await load()
    }
}

/// An item's row: its icon, title, what it is, and the icons of the apps that use it.
private struct VaultRow: View {
    @Environment(Store.self) private var store
    var item: VaultItem

    var body: some View {
        HStack(spacing: 12) {
            ItemIcon(item: item, size: 36)
            VStack(alignment: .leading, spacing: 2) {
                Text(item.title).lineLimit(1).truncationMode(.middle)
                if let line = item.line {
                    Text(line).font(.callout).foregroundStyle(.secondary).lineLimit(1)
                }
            }
            Spacer(minLength: 8)
            HStack(spacing: 4) {
                ForEach(item.apps.prefix(6), id: \.self) { app in
                    AppTile(app: store.app(app), seed: app, size: 20).help(store.app(app)?.label ?? app)
                }
            }
            .accessibilityLabel(appNames(item.apps, store: store))
        }
        .padding(.vertical, 2)
    }
}

/// An item's icon: the logo of the site it's for, else the icon of the app that made or uses it, else a key.
struct ItemIcon: View {
    @Environment(Store.self) private var store
    var title: String
    var image: String?
    var app: String?
    var size: CGFloat

    init(item: VaultItem, size: CGFloat) {
        self.init(title: item.title, image: item.image, app: item.app ?? item.apps.first, size: size)
    }

    init(title: String, image: String? = nil, app: String?, size: CGFloat) {
        self.title = title
        self.image = image
        self.app = app
        self.size = size
    }

    var body: some View {
        if let image {
            AppTile(icon: nil, seed: title, image: image, size: size)
        } else if let app, let info = store.app(app) {
            AppTile(app: info, seed: app, size: size)
        } else {
            AppTile(icon: "key.fill", seed: title, color: "#8e8e93", size: size)
        }
    }
}

/// "Gmail and Drive": the apps' names, as a person reads them.
@MainActor
func appNames(_ apps: [String], store: Store) -> String {
    apps.map { store.app($0)?.label ?? $0 }.formatted(.list(type: .and))
}

/// A field's name as a person reads it: `client_id` is "Client ID".
extension Store {
    /// Deletes an item, signing an account out first as connectors do (SPEC.md "Connectors"): each app using it, or
    /// whose sign-in made it, that lists it among its `accounts` (by its title or a plain field) signs that account out,
    /// so what the app keeps elsewhere goes too (a sidecar's link) and listing accounts doesn't make the item again.
    func delete(_ item: VaultItem) async throws {
        struct Listed: Decodable { var account: String }
        if item.isAccount {
            let known = Set([item.title] + item.fields.compactMap { $0.secret ? nil : $0.value })
            for name in Set(item.apps + [item.app].compactMap { $0 }).sorted() {
                guard let app = self.app(name), app.has(op: "accounts"), app.has(op: "sign_out") else { continue }
                let listed = try JSONDecoder().decode([Listed].self, from: try await node.call(app: name, op: "accounts", input: Data("{}".utf8)))
                guard let account = listed.first(where: { known.contains($0.account) }) else { continue }
                _ = try await node.call(app: name, op: "sign_out", input: try JSONEncoder().encode(["account": account.account]))
            }
            // Signing out may have let go of it already.
            guard try await node.vault(scope: item.scope).contains(where: { $0.id == item.id }) else { return }
        }
        try await node.setItem(scope: item.scope, id: item.id, ["deleted": true])
    }
}

func fieldLabel(_ name: String) -> String {
    let words = name.split(whereSeparator: { "_-.".contains($0) }).map { word -> String in
        let w = word.lowercased()
        return ["id", "url", "api", "pin", "otp"].contains(w) ? w.uppercased() : w
    }
    guard let first = words.first else { return name }
    return ([first.prefix(1).uppercased() + first.dropFirst()] + words.dropFirst()).joined(separator: " ")
}

/// One item: its fields (a plain one to copy, a secret one only to replace), where they're sent, the apps that want
/// it, each with a switch, and deleting it (signing out, for an account a sign-in made).
struct VaultItemView: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State var item: VaultItem
    var changed: () async -> Void
    @State private var hosts = ""
    @State private var editing: VaultItem.Field?
    @State private var addingField = false
    @State private var deleting = false
    @State private var error: String?

    init(item: VaultItem, changed: @escaping () async -> Void) {
        _item = State(initialValue: item)
        self.changed = changed
    }

    var body: some View {
        Form {
            Section {
                VStack(spacing: 8) {
                    ItemIcon(item: item, size: 72)
                    Text(item.title).font(.title2.weight(.semibold)).multilineTextAlignment(.center).textSelection(.enabled)
                    if let page = item.scope_name {
                        Text(page).foregroundStyle(.secondary)
                    }
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 8)
            }
            Section {
                ForEach(item.fields) { field in
                    LabeledContent(fieldLabel(field.name)) {
                        HStack(spacing: 12) {
                            if field.secret {
                                Text("••••••••").foregroundStyle(.secondary)
                                Button("Replace") { editing = field }.buttonStyle(.borderless)
                            } else {
                                Text(field.value ?? "").textSelection(.enabled).lineLimit(2).truncationMode(.middle)
                                Button("Copy") { copy(field.value ?? "") }.buttonStyle(.borderless)
                            }
                        }
                    }
                    .contextMenu {
                        if !field.secret { Button("Edit", systemImage: "pencil") { editing = field } }
                        Button("Remove", systemImage: "minus.circle", role: .destructive) { Task { await save(["fields": [field.name: NSNull()]]) } }
                    }
                }
                Button("Add Field", systemImage: "plus") { addingField = true }.buttonStyle(.borderless)
            }
            Section("Sent only to") {
                TextField("Sites", text: $hosts, prompt: Text("api.example.com"))
                    .autocorrectionDisabled()
                    #if os(iOS)
                    .textInputAutocapitalization(.never)
                    .keyboardType(.URL)
                    #endif
                    .onSubmit { Task { await saveHosts() } }
            }
            let wanting = usedBy
            if !wanting.isEmpty {
                Section("Used by") {
                    ForEach(wanting, id: \.self) { app in
                        Toggle(isOn: Binding(get: { item.apps.contains(app) }, set: { on in Task { await switchApp(app, on) } })) {
                            HStack(spacing: 10) {
                                AppTile(app: store.app(app), seed: app, size: 28)
                                Text(store.app(app)?.label ?? app)
                            }
                        }
                    }
                }
            }
            Section {
                Button(item.isAccount ? "Sign Out" : "Delete Item", role: .destructive) { deleting = true }
            }
            if let error {
                Label(error, systemImage: "exclamationmark.circle.fill").foregroundStyle(.red)
            }
        }
        .formStyle(.grouped)
        .navigationTitle(item.title)
        #if os(iOS)
        .navigationBarTitleDisplayMode(.inline)
        #endif
        .onAppear { hosts = item.hosts.joined(separator: ", ") }
        .sheet(item: $editing) { field in
            FieldSheet(name: field.name, secret: field.secret, editingName: false) { _, value, _ in
                await save(["fields": [field.name: ["value": value]]])
            }
        }
        .sheet(isPresented: $addingField) {
            FieldSheet(name: "", secret: true, editingName: true) { name, value, secret in
                await save(["fields": [name: ["value": value, "secret": secret]]])
            }
        }
        .confirmationDialog(item.isAccount ? "Sign out of \(item.title)?" : "Delete \(item.title)?", isPresented: $deleting, titleVisibility: .visible) {
            Button(item.isAccount ? "Sign Out" : "Delete", role: .destructive) {
                Task {
                    do {
                        try await store.delete(item)
                        await changed()
                        dismiss()
                    } catch {
                        self.error = error.localizedDescription
                    }
                }
            }
        } message: {
            Text(item.apps.isEmpty ? "It can’t be undone." : "\(appNames(item.apps, store: store)) stop using it.")
        }
    }

    /// The apps that want it or use it, those using it first.
    private var usedBy: [String] {
        var seen = Set<String>()
        return (item.apps + (item.wanted_by ?? [])).filter { seen.insert($0).inserted }
    }

    private func switchApp(_ app: String, _ on: Bool) async {
        var apps = item.apps.filter { $0 != app }
        if on { apps.append(app) }
        await save(["apps": apps])
    }

    private func saveHosts() async {
        let list = hosts.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces).lowercased() }.filter { !$0.isEmpty }
        guard list != item.hosts else { return }
        await save(["hosts": list])
    }

    /// Changes the item, then shows it as it is now. Whether it worked.
    @discardableResult
    private func save(_ change: sending [String: Any?]) async -> Bool {
        error = nil
        do {
            try await store.node.setItem(scope: item.scope, id: item.id, change)
            if let fresh = try await store.node.vault(scope: item.scope).first(where: { $0.id == item.id }) {
                item = fresh
                hosts = fresh.hosts.joined(separator: ", ")
            }
            await changed()
            return true
        } catch {
            self.error = error.localizedDescription
            hosts = item.hosts.joined(separator: ", ")
            return false
        }
    }

    private func copy(_ text: String) {
        #if os(macOS)
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
        #else
        UIPasteboard.general.string = text
        #endif
    }
}

/// A field's new value (a secret one typed unseen), or a new field with its name.
private struct FieldSheet: View {
    @Environment(\.dismiss) private var dismiss
    @State var name: String
    @State var secret: Bool
    var editingName: Bool
    var save: (String, String, Bool) async -> Bool
    @State private var value = ""

    init(name: String, secret: Bool, editingName: Bool, save: @escaping (String, String, Bool) async -> Bool) {
        _name = State(initialValue: name)
        _secret = State(initialValue: secret)
        self.editingName = editingName
        self.save = save
    }

    var body: some View {
        NavigationStack {
            Form {
                if editingName {
                    TextField("Name", text: $name, prompt: Text("token"))
                        .autocorrectionDisabled()
                        #if os(iOS)
                        .textInputAutocapitalization(.never)
                        #endif
                    Toggle("Secret", isOn: $secret)
                }
                if secret {
                    SecureField(editingName ? "Value" : fieldLabel(name), text: $value).textContentType(.password)
                } else {
                    TextField(editingName ? "Value" : fieldLabel(name), text: $value)
                }
            }
            .formStyle(.grouped)
            .navigationTitle(editingName ? String(localized: "New Field") : fieldLabel(name))
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { Task { if await save(name.trimmingCharacters(in: .whitespaces), value, secret) { dismiss() } } }
                        .disabled(value.isEmpty || name.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
        }
        #if os(macOS)
        .frame(width: 400, height: editingName ? 240 : 160)
        #endif
    }
}

/// A new item a person types: its page, title, fields (each secret or not) and the sites they're sent to.
private struct NewItemSheet: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State var scope: String
    var saved: () async -> Void
    @State private var title = ""
    @State private var fields: [Draft] = [Draft(name: "", secret: true)]
    @State private var hosts = ""
    @State private var error: String?

    struct Draft: Identifiable {
        var id = UUID()
        var name: String
        var value = ""
        var secret: Bool
    }

    var body: some View {
        let pages = store.allPages.filter { store.canArrange($0.scope) }
        NavigationStack {
            Form {
                Section {
                    TextField("Title", text: $title, prompt: Text("Example API key"))
                    if pages.count > 1 {
                        Picker("Page", selection: $scope) {
                            ForEach(pages) { Text($0.title).tag($0.scope) }
                        }
                    }
                }
                ForEach($fields) { $field in
                    Section {
                        TextField("Name", text: $field.name, prompt: Text("token"))
                            .autocorrectionDisabled()
                            #if os(iOS)
                            .textInputAutocapitalization(.never)
                            #endif
                        if field.secret {
                            SecureField("Value", text: $field.value)
                        } else {
                            TextField("Value", text: $field.value)
                        }
                        Toggle("Secret", isOn: $field.secret)
                    }
                }
                Section {
                    Button("Add Field", systemImage: "plus") { fields.append(Draft(name: "", secret: false)) }.buttonStyle(.borderless)
                }
                Section("Sent only to") {
                    TextField("Sites", text: $hosts, prompt: Text("api.example.com"))
                        .autocorrectionDisabled()
                        #if os(iOS)
                        .textInputAutocapitalization(.never)
                        .keyboardType(.URL)
                        #endif
                }
                if let error {
                    Label(error, systemImage: "exclamationmark.circle.fill").foregroundStyle(.red)
                }
            }
            .formStyle(.grouped)
            .navigationTitle("New Item")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { Task { await save() } }
                        .disabled(title.trimmingCharacters(in: .whitespaces).isEmpty || !fields.contains { !$0.name.isEmpty && !$0.value.isEmpty })
                }
            }
        }
        #if os(macOS)
        .frame(width: 440, height: 480)
        #endif
    }

    private func save() async {
        var given: [String: Any] = [:]
        for field in fields where !field.name.trimmingCharacters(in: .whitespaces).isEmpty && !field.value.isEmpty {
            given[field.name.trimmingCharacters(in: .whitespaces)] = ["value": field.value, "secret": field.secret]
        }
        let sites = hosts.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces).lowercased() }.filter { !$0.isEmpty }
        do {
            try await store.node.setItem(scope: scope, id: nil, ["title": title.trimmingCharacters(in: .whitespaces), "fields": given, "hosts": sites, "apps": [String]()])
            await saved()
            dismiss()
        } catch {
            self.error = error.localizedDescription
        }
    }
}
