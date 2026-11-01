import SwiftUI
import UniformTypeIdentifiers

/// A request for an item an app may use: who asks and why, then the person's items that have the fields it wants, to
/// pick one, or a new one, whose values go to the node and nowhere else. The asker never sees a secret.
/// With `ask`, the shell's own request for something of the install's that isn't an app (push), made here rather than
/// as an entry: the item picked or made is switched on for `ask.app` directly.
struct VaultRequestSheet: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    var scope: String
    var id: String
    var ask: VaultRequest? = nil
    @State private var request: VaultRequest?
    /// The file chosen for each file field, by name, and the field choosing one.
    @State private var files: [String: String] = [:]
    @State private var choosing: String?
    /// The item picked, or nil for a new one.
    @State private var picked: String?
    @State private var title = ""
    @State private var values: [String: String] = [:]
    @State private var error: String?
    @State private var busy = false

    var body: some View {
        NavigationStack {
            Form {
                if let request {
                    let app = store.app(request.app)
                    Section {
                        HStack(spacing: 12) {
                            AppTile(app: app, seed: request.app, size: 44)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(app.map { "\($0.label) wants \(request.label)" } ?? request.label).font(.headline)
                                if let why = request.why, !why.isEmpty {
                                    Text(why).foregroundStyle(.secondary)
                                }
                            }
                        }
                        .padding(.vertical, 2)
                    } footer: {
                        Text(request.agent.map { _ in "\(request.asker) · \(request.scope_name)" } ?? request.scope_name)
                    }
                    if request.waiting {
                        Section {
                            ForEach(request.matches ?? []) { match in
                                choice(id: match.id) {
                                    ItemIcon(title: match.title, app: match.apps?.first, size: 30)
                                    VStack(alignment: .leading, spacing: 1) {
                                        Text(match.title)
                                        if let type = match.type, !type.isEmpty { Text(type).font(.callout).foregroundStyle(.secondary) }
                                    }
                                }
                            }
                            choice(id: nil) {
                                Image(systemName: "plus.circle.fill").font(.title2).foregroundStyle(Color.accentColor).frame(width: 30)
                                Text("New Item")
                            }
                        }
                        if picked == nil {
                            Section {
                                TextField("Title", text: $title)
                                ForEach(request.wanted) { field in
                                    if field.file == true {
                                        Button { choosing = field.name } label: {
                                            LabeledContent(fieldLabel(field.name), value: files[field.name] ?? String(localized: "Choose File…"))
                                                .contentShape(.rect)
                                        }
                                        .buttonStyle(.plain)
                                    } else if field.secret {
                                        SecureField(fieldLabel(field.name), text: binding(field.name)).textContentType(.password)
                                    } else {
                                        TextField(fieldLabel(field.name), text: binding(field.name))
                                            .autocorrectionDisabled()
                                            #if os(iOS)
                                            .textInputAutocapitalization(.never)
                                            #endif
                                    }
                                }
                            } footer: {
                                if !request.hosts.isEmpty { Text("Sent only to \(request.hosts.formatted())") }
                            }
                        }
                        if ask == nil {
                            Section {
                                Button("Decline", role: .destructive) { answer(declined: true) }.disabled(busy)
                            }
                        }
                        if let error {
                            Label(error, systemImage: "exclamationmark.circle.fill").foregroundStyle(.red)
                        }
                    } else {
                        Section {
                            Text(stateText(request)).foregroundStyle(.secondary)
                        }
                    }
                } else if let error {
                    ContentUnavailableView("Can’t Find This Request", systemImage: "lock.slash", description: Text(error))
                } else {
                    ProgressView().frame(maxWidth: .infinity)
                }
            }
            .formStyle(.grouped)
            .navigationTitle("Vault")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                if request?.waiting == true {
                    ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                    ToolbarItem(placement: .confirmationAction) {
                        Button(ask == nil ? "Allow" : "Save") { answer() }.disabled(!ready || busy)
                    }
                } else {
                    ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
                }
            }
            .task { await load() }
            .fileImporter(isPresented: Binding(get: { choosing != nil }, set: { if !$0 { choosing = nil } }), allowedContentTypes: [.data]) { result in
                guard let name = choosing, case .success(let url) = result else { return }
                let scoped = url.startAccessingSecurityScopedResource()
                defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                guard let text = try? String(contentsOf: url, encoding: .utf8) else {
                    error = String(localized: "\(url.lastPathComponent) isn’t text.")
                    return
                }
                values[name] = text
                files[name] = url.lastPathComponent
            }
        }
        #if os(macOS)
        .frame(width: 460, height: 460)
        #endif
    }

    private func choice(id: String?, @ViewBuilder label: () -> some View) -> some View {
        Button { picked = id } label: {
            HStack(spacing: 12) {
                label()
                Spacer()
                if picked == id { Image(systemName: "checkmark").foregroundStyle(Color.accentColor).fontWeight(.semibold) }
            }
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
    }

    private func binding(_ name: String) -> Binding<String> {
        Binding(get: { values[name] ?? "" }, set: { values[name] = $0 })
    }

    /// Whether Allow has what it needs: an item picked, or every field of a new one.
    private var ready: Bool {
        guard let request else { return false }
        return picked != nil || request.wanted.allSatisfy { !(values[$0.name] ?? "").isEmpty }
    }

    private func stateText(_ request: VaultRequest) -> String {
        switch request.state {
        case "set": String(localized: "Allowed.")
        case "declined": String(localized: "Declined.")
        default: String(localized: "This request expired. Ask \(request.asker) to ask again.")
        }
    }

    private func load() async {
        do {
            if var ask {
                // The items here that have every field it wants, those already switched on for it first.
                let names = Set(ask.wanted.map(\.name))
                let items = try await store.node.vault(scope: ask.scope).filter { names.isSubset(of: Set($0.fields.map(\.name))) }
                ask.matches = items.sorted { $0.apps.contains(ask.app) && !$1.apps.contains(ask.app) }
                    .map { VaultRequest.Match(id: $0.id, title: $0.title, type: $0.type, domain: $0.domain, apps: $0.apps) }
                request = ask
                picked = ask.matches?.first?.id
                title = ask.label
                return
            }
            request = try await store.node.vaultRequests(scope: scope, id: id).first
            if let request {
                picked = request.matches?.first?.id
                title = request.label
            } else {
                error = String(localized: "It isn’t for you, or it’s from another install.")
            }
        } catch { self.error = error.localizedDescription }
    }

    private func answer(declined: Bool = false) {
        guard let request, !busy, declined || ready else { return }
        busy = true
        error = nil
        Task {
            defer { busy = false }
            do {
                if let ask {
                    if let picked, let match = request.matches?.first(where: { $0.id == picked }) {
                        let apps = (match.apps ?? []).filter { $0 != ask.app } + [ask.app]
                        _ = try await store.node.setItem(scope: ask.scope, id: picked, ["apps": apps])
                    } else {
                        let name = title.trimmingCharacters(in: .whitespaces)
                        let fields = Dictionary(uniqueKeysWithValues: ask.wanted.map { ($0.name, ["value": values[$0.name] ?? "", "secret": $0.secret] as [String: Any]) })
                        _ = try await store.node.setItem(scope: ask.scope, id: nil, ["title": name.isEmpty ? ask.label : name, "fields": fields, "hosts": ask.hosts, "apps": [ask.app]])
                    }
                } else if declined {
                    try await store.node.answer(request: request.id, scope: request.scope, declined: true)
                } else if let picked {
                    try await store.node.answer(request: request.id, scope: request.scope, item: picked)
                } else {
                    let name = title.trimmingCharacters(in: .whitespaces)
                    try await store.node.answer(request: request.id, scope: request.scope, title: name.isEmpty ? nil : name, fields: values)
                }
                values = [:]
                try? await store.refreshState()
                dismiss()
            } catch { self.error = error.localizedDescription }
        }
    }
}
