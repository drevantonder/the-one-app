import SwiftUI

/// What to paste into an agent so it connects itself: read the skills this server serves on the tailnet, or, where
/// it serves none, run the command line's guide.
func agentPrompt(name: String, cli: String, guide: String?) -> String {
    if let guide {
        "Connect yourself to \(name): read \(guide) and set it up for me."
    } else {
        "Connect yourself to \(name): run \"\(cli) guide\" in a terminal and set it up for me."
    }
}

/// The prompt, and a button that copies it.
struct AskYourAgent: View {
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

/// Ask Your Agent, on its own, for the first screen.
struct AskYourAgentSheet: View {
    var prompt: String
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    AskYourAgent(prompt: prompt)
                }
            }
            .formStyle(.grouped)
            .navigationTitle("Ask Your Agent")
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
