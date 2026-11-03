import SwiftUI

/// What to paste into an AI so it connects itself, before there's a node to ask (`agent_prompt` in its state, from
/// src/state.rs, says it once there is): the guide at your One App's `address`, or the command line's guide.
func agentPrompt(address: String?) -> String {
    let yours = Bundle.main.object(forInfoDictionaryKey: "Yours") as? String ?? "", cli = Bundle.main.object(forInfoDictionaryKey: "CLI") as? String ?? ""
    if let url = address.flatMap({ URL(string: $0.contains("://") ? $0 : "https://\($0)") }), let host = url.host() {
        let base = "\(url.scheme ?? "https")://\(host)\(url.port.map { ":\($0)" } ?? "")"
        return "Read \(base)/llms.txt and connect yourself to \(yours)."
    }
    return "Run \"\(cli) guide\" in a terminal, read what it says, and connect yourself to \(yours)."
}

/// The prompt, and a button that copies it.
struct ConnectYourAI: View {
    var prompt: String
    @State private var copied = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(prompt)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
            Button(copied ? "Copied" : "Copy", systemImage: copied ? "checkmark" : "doc.on.doc") {
                #if os(macOS)
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(prompt, forType: .string)
                #else
                UIPasteboard.general.string = prompt
                #endif
                copied = true
                Task {
                    try? await Task.sleep(for: .seconds(2))
                    copied = false
                }
            }
        }
        .padding(.vertical, 6)
    }
}

/// Connect Your AI, on its own, for the first screen.
struct ConnectYourAISheet: View {
    var prompt: String
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    ConnectYourAI(prompt: prompt)
                }
            }
            .formStyle(.grouped)
            .navigationTitle("Connect Your AI")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Done") { dismiss() } }
            }
        }
        #if os(macOS)
        .frame(width: 440, height: 220)
        #else
        .presentationDetents([.medium])
        #endif
    }
}
