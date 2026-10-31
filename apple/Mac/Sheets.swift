import SwiftUI
import AppKit

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

struct JoinSheet: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var code = ""
    @State private var error: String?
    @State private var joining = false

    var body: some View {
        SheetLayout(title: "Join with a Pairing Code", message: "Paste the pairing code another device shows. Each code works once.") {
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
