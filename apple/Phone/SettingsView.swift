import SwiftUI

struct SettingsView: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var renaming = false
    @State private var error: String?

    var body: some View {
        @Bindable var store = store
        NavigationStack {
            Form {
                Section {
                    HStack {
                        TextField("Name", text: $name)
                            .submitLabel(.done)
                            .onSubmit { rename() }
                        if renaming { ProgressView() }
                    }
                } header: {
                    Text("This iPhone")
                } footer: {
                    if let error {
                        Label(error, systemImage: "exclamationmark.circle.fill").foregroundStyle(.red)
                    } else {
                        Text("Other devices in the space see this iPhone by this name.")
                    }
                }

                if let space = store.space {
                    Section("Space") {
                        LabeledContent("Name") {
                            HStack(spacing: 8) {
                                SpaceMark(name: space.title, size: 22)
                                Text(space.title)
                            }
                        }
                        LabeledContent("Role", value: space.role == "admin" ? String(localized: "Admin") : String(localized: "Member"))
                        LabeledContent("Devices", value: "\(space.devices.count)")
                        LabeledContent("Entries", value: space.entries.formatted())
                    }
                }

                if let state = store.state {
                    Section {
                        if let host = state.host {
                            LabeledContent("Connected Through", value: displayName(host.name))
                        }
                        LabeledContent("Device ID") {
                            Text(state.device.id.prefix(12))
                                .font(.body.monospaced())
                                .textSelection(.enabled)
                        }
                    } footer: {
                        Text("This iPhone keeps no copy of the space. A device that does answers for it.")
                    }
                }
            }
            .navigationTitle("Settings")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done", systemImage: "checkmark", role: .confirm) { rename(closing: true) }
                }
            }
            .onAppear { name = store.state?.device.name ?? "" }
            .onChange(of: name) { error = nil }
        }
    }

    private func rename(closing: Bool = false) {
        let new = name.trimmingCharacters(in: .whitespaces)
        guard !new.isEmpty, new != store.state?.device.name else { return closing ? dismiss() : () }
        guard !renaming else { return }
        renaming = true
        Task {
            defer { renaming = false }
            do {
                try await store.renameDevice(new)
                if closing { dismiss() }
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}
