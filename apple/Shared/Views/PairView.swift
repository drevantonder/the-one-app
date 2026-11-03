import SwiftUI

/// A pairing code, for another of this user's computers.
struct PairSheet: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var code: String?
    @State private var error: String?
    @State private var copied = false

    var body: some View {
        VStack(spacing: 18) {
            VStack(spacing: 6) {
                Text("Pair a Computer").font(.title2.weight(.semibold))
                Text("It joins as you, with everything you can see and do.")
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
            if let error {
                Label(error, systemImage: "exclamationmark.circle.fill").foregroundStyle(.red)
            } else if code == nil {
                Later(.milliseconds(400)) { ProgressView() }
            }
            HStack(spacing: 10) {
                Text(code ?? " ")
                    .font(.system(.callout, design: .monospaced))
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .textSelection(.enabled)
                    .foregroundStyle(.secondary)
                Button(copied ? "Copied" : "Copy", systemImage: copied ? "checkmark" : "doc.on.doc", action: copy)
                    .disabled(code == nil)
            }
            .padding(.leading, 12)
            .padding(.trailing, 4)
            .padding(.vertical, 4)
            .background(.quaternary.opacity(0.5), in: .capsule)
            Text("On the other computer, choose Join with a Pairing Code, or run \(store.config?.cli ?? "") join with the code. It works once, for seven days.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            Button("Done") { dismiss() }
                .keyboardShortcut(.defaultAction)
        }
        .padding(24)
        #if os(macOS)
        .frame(width: 440)
        #endif
        .task {
            do { code = try await store.node.pair() } catch { self.error = error.localizedDescription }
        }
    }

    private func copy() {
        guard let code else { return }
        #if os(macOS)
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(code, forType: .string)
        #else
        UIPasteboard.general.string = code
        #endif
        copied = true
    }
}

/// Reordering and hiding group pages, for this user on this device. Their own page and everyone's stay put.
struct PagesEditor: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                Section {
                    ForEach(store.allPages.filter { $0.group != nil }) { page in
                        Toggle(page.title, isOn: Binding(get: { !store.hiddenPages.contains(page.scope) }, set: { store.setHidden(page.scope, !$0) }))
                    }
                    .onMove { store.movePages(from: $0, to: $1) }
                } footer: {
                    Text("Drag to reorder; switch one off to hide it.")
                }
            }
            #if os(iOS)
            .environment(\.editMode, .constant(.active))
            #endif
            .navigationTitle("Pages")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
            }
        }
        #if os(macOS)
        .frame(width: 360, height: 360)
        #endif
    }
}
