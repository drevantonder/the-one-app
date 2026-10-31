import CoreImage.CIFilterBuiltins
import SwiftUI

/// A pairing code, as a QR code and text: for another of this user's devices, or, from Users, for someone new.
struct PairSheet: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    /// Someone new, named, as an admin or member.
    var newUser: (name: String, role: String)?
    @State private var code: String?
    @State private var error: String?
    @State private var copied = false

    var body: some View {
        VStack(spacing: 18) {
            VStack(spacing: 6) {
                Text(newUser.map { "Invite \($0.name)" } ?? String(localized: "Pair a Phone or Computer")).font(.title2.weight(.semibold))
                Text(newUser == nil
                     ? "It joins as you, with everything you can see and do."
                     : "They join \(store.install?.name ?? "the install") as a new user, on the device that scans this.")
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
            ZStack {
                RoundedRectangle(cornerRadius: 18, style: .continuous)
                    .fill(.white)
                    .shadow(color: .black.opacity(0.12), radius: 10, y: 3)
                if let code, let qr = qrCode(code) {
                    Image(decorative: qr, scale: 1)
                        .interpolation(.none)
                        .resizable()
                        .scaledToFit()
                        .padding(16)
                } else if let error {
                    Label(error, systemImage: "exclamationmark.circle.fill").foregroundStyle(.red).padding()
                } else {
                    ProgressView()
                }
            }
            .frame(width: 236, height: 236)
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
            Text("Scan it with the other device’s camera, or run \(store.brand?.cli ?? "") join with the code there. It works once, for seven days.")
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
            do { code = try await store.node.pair(newUser: newUser) } catch { self.error = error.localizedDescription }
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

    private func qrCode(_ text: String) -> CGImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(text.utf8)
        filter.correctionLevel = "M"
        guard let image = filter.outputImage else { return nil }
        return CIContext().createCGImage(image, from: image.extent)
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
                    Text("Drag to reorder your groups’ pages; switch one off to hide it. You stays first and Everyone last.")
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
