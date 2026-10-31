import SwiftUI

/// A request that this user set a secret: who asks, why, whose scope keeps it and where it's sent, and a field for its
/// value, which goes to the node and nowhere else. The asker never sees it.
struct SecretRequestSheet: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    var scope: String
    var id: String
    @State private var request: SecretRequest?
    @State private var value = ""
    @State private var error: String?
    @State private var busy = false

    var body: some View {
        NavigationStack {
            Form {
                if let request {
                    Section {
                        VStack(alignment: .leading, spacing: 6) {
                            Text("\(request.asker) asks for \(appLabel(request))’s \(Text(request.name).font(.body.monospaced()))")
                                .font(.headline)
                            if let why = request.why, !why.isEmpty {
                                Text(why).foregroundStyle(.secondary)
                            }
                        }
                        .padding(.vertical, 2)
                        LabeledContent("Saved for", value: savedFor(request))
                        LabeledContent("Sent only to", value: request.hosts.formatted())
                    }
                    if request.waiting {
                        Section {
                            SecureField("Value", text: $value, prompt: Text(request.name))
                                .textContentType(.password)
                                .autocorrectionDisabled()
                                .onSubmit { answer(value) }
                        } footer: {
                            if let error {
                                Label(error, systemImage: "exclamationmark.circle.fill").foregroundStyle(.red)
                            } else {
                                Text("Nothing shows it again, not even to you, and \(request.asker) never sees it: \(appLabel(request)) gets it only in requests to its hosts.")
                            }
                        }
                        Section {
                            Button("Decline", role: .destructive) { answer(nil) }.disabled(busy)
                        }
                    } else {
                        Section {
                            Text(stateText(request)).foregroundStyle(.secondary)
                        }
                    }
                } else if let error {
                    ContentUnavailableView("Can’t Find This Request", systemImage: "key.slash", description: Text(error))
                } else {
                    ProgressView().frame(maxWidth: .infinity)
                }
            }
            .formStyle(.grouped)
            .navigationTitle("Secret Requested")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                if request?.waiting == true {
                    ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Save") { answer(value) }.disabled(value.isEmpty || busy)
                    }
                } else {
                    ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
                }
            }
            .task { await load() }
        }
        #if os(macOS)
        .frame(width: 440, height: 380)
        #endif
    }

    private func appLabel(_ request: SecretRequest) -> String { store.app(request.app)?.label ?? request.app }

    /// Whose scope keeps it, and so who uses it.
    private func savedFor(_ request: SecretRequest) -> String {
        if request.scope == "global" { return String(localized: "\(request.scope_name): everyone in the install") }
        if request.scope.hasPrefix("group:") { return String(localized: "\(request.scope_name): everyone in the group") }
        return String(localized: "\(request.scope_name): only you")
    }

    private func stateText(_ request: SecretRequest) -> String {
        switch request.state {
        case "set": String(localized: "It’s been set.")
        case "declined": String(localized: "It was declined.")
        default: String(localized: "It expired: requests wait a day. Ask \(request.asker) to ask again.")
        }
    }

    private func load() async {
        do {
            request = try await store.node.secretRequests(scope: scope, id: id).first
            if request == nil { error = String(localized: "It isn’t for you, or it’s from another install.") }
        } catch { self.error = error.localizedDescription }
    }

    private func answer(_ value: String?) {
        guard let request, !busy, value.map({ !$0.isEmpty }) ?? true else { return }
        busy = true
        error = nil
        Task {
            defer { busy = false }
            do {
                try await store.node.answer(secretRequest: request.id, scope: request.scope, value: value)
                self.value = ""
                try? await store.refreshState()
                dismiss()
            } catch { self.error = error.localizedDescription }
        }
    }
}
