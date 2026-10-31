import CoreImage.CIFilterBuiltins
import SwiftUI
import AppKit

struct SheetView: View {
    var sheet: Sheet

    var body: some View {
        switch sheet {
        case .pair: PairSheet()
        case .join: JoinSheet()
        case .newSpace: NewSpaceSheet()
        }
    }
}

private struct SheetLayout<Content: View, Actions: View>: View {
    var title: LocalizedStringKey
    var message: LocalizedStringKey
    @ViewBuilder var content: Content
    @ViewBuilder var actions: Actions

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            VStack(alignment: .leading, spacing: 6) {
                Text(title).font(.title2.weight(.semibold))
                Text(message).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
            content
            HStack {
                Spacer()
                actions
            }
        }
        .padding(24)
        .frame(width: 440)
    }
}

private struct ErrorLine: View {
    var error: String?

    var body: some View {
        if let error {
            Label(error, systemImage: "exclamationmark.circle.fill")
                .font(.callout)
                .foregroundStyle(.red)
        }
    }
}

struct PairSheet: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var newUser = false
    @State private var code: String?
    @State private var error: String?
    @State private var copied = false

    var body: some View {
        VStack(spacing: 18) {
            VStack(spacing: 6) {
                Text("Pair a Device").font(.title2.weight(.semibold))
                Text("Bring another device into \(store.space?.title ?? "this space").")
                    .foregroundStyle(.secondary)
            }
            Picker("Who is it for?", selection: $newUser) {
                Text("One of My Devices").tag(false)
                Text("Someone New").tag(true)
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .fixedSize()
            Text(newUser
                 ? "They join as a new user and a member of this space."
                 : "It joins as you, with everything you can see and do.")
                .font(.callout)
                .foregroundStyle(.secondary)
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
                    ErrorLine(error: error).padding()
                } else {
                    ProgressView()
                }
            }
            .frame(width: 236, height: 236)
            .padding(.vertical, 4)
            HStack(spacing: 10) {
                Text(code ?? " ")
                    .font(.system(.callout, design: .monospaced))
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .textSelection(.enabled)
                    .foregroundStyle(.secondary)
                Button(copied ? "Copied" : "Copy", systemImage: copied ? "checkmark" : "doc.on.doc", action: copy)
                    .disabled(code == nil)
                    .contentTransition(.symbolEffect(.replace))
            }
            .padding(.leading, 12)
            .padding(.trailing, 4)
            .padding(.vertical, 4)
            .background(.quaternary.opacity(0.5), in: .capsule)
            Text("Scan it with the other device, or run \(store.brand?.cli ?? "") join with the code there. It works once.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            HStack {
                Spacer()
                Button("Done") { dismiss() }
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(24)
        .frame(width: 440)
        .task(id: newUser) {
            code = nil; error = nil; copied = false
            do { code = try await store.pair(newUser: newUser) } catch { self.error = error.localizedDescription }
        }
    }

    private func copy() {
        guard let code else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(code, forType: .string)
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

struct JoinSheet: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var code = ""
    @State private var error: String?
    @State private var joining = false

    var body: some View {
        SheetLayout(title: "Join with a Pairing Code", message: "Paste the pairing code your other device shows. Each code works once.") {
            TextField("Pairing Code", text: $code, prompt: Text("Pairing code"))
                .font(.system(.body, design: .monospaced))
                .textFieldStyle(.roundedBorder)
                .controlSize(.large)
                .onChange(of: code) { error = nil }
            ErrorLine(error: error)
        } actions: {
            if joining { ProgressView().controlSize(.small).padding(.trailing, 4) }
            Button("Cancel", role: .cancel) { dismiss() }
                .keyboardShortcut(.cancelAction)
            Button("Join") {
                joining = true
                Task {
                    defer { joining = false }
                    do { try await store.join(code); dismiss() } catch { self.error = error.localizedDescription }
                }
            }
            .keyboardShortcut(.defaultAction)
            .disabled(code.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || joining)
        }
        .onAppear {
            let clip = NSPasteboard.general.string(forType: .string) ?? ""
            let trimmed = clip.trimmingCharacters(in: .whitespacesAndNewlines)
            if trimmed.count > 60, trimmed.allSatisfy({ $0.isLetter || $0.isNumber }) { code = trimmed }
        }
    }
}

struct NewSpaceSheet: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var error: String?

    var body: some View {
        SheetLayout(title: "New Space", message: "A space holds devices, their apps and their agents. Start with your own, then pair others.") {
            TextField("Name", text: $name, prompt: Text("Home"))
                .textFieldStyle(.roundedBorder)
                .onSubmit(create)
            ErrorLine(error: error)
        } actions: {
            Button("Cancel", role: .cancel) { dismiss() }
                .keyboardShortcut(.cancelAction)
            Button("Create", action: create)
                .keyboardShortcut(.defaultAction)
                .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty)
        }
    }

    private func create() {
        Task {
            do { try await store.createSpace(name); dismiss() } catch { self.error = error.localizedDescription }
        }
    }
}
