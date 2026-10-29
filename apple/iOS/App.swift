import SwiftUI

// A phone or iPad can't run a node, so the app talks to one over Tailscale.
@main
struct OneApp: App {
    @AppStorage("node") private var node = "mac:4747"

    var body: some Scene {
        WindowGroup {
            Root(node: $node)
        }
    }
}

struct Root: View {
    @Binding var node: String
    @StateObject private var shell: Shell
    @State private var editing = false

    init(node: Binding<String>) {
        _node = node
        _shell = StateObject(wrappedValue: Shell(node: nodeURL(node.wrappedValue)))
    }

    var body: some View {
        ShellView(shell: shell, settings: AnyView(Button("Node", systemImage: "server.rack") { editing = true }))
        .sheet(isPresented: $editing) {
            NavigationStack {
                Form {
                    Section {
                        TextField("mac:4747", text: $node)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .keyboardType(.URL)
                    } header: {
                        Text("Node")
                    } footer: {
                        Text("A device of yours running The One App, by its Tailscale name. Tailscale must be on.")
                    }
                }
                .navigationTitle("Node")
                .toolbar {
                    Button("Done") {
                        editing = false
                        shell.connect(to: nodeURL(node))
                    }
                }
            }
        }
    }
}

func nodeURL(_ address: String) -> URL {
    let trimmed = address.trimmingCharacters(in: .whitespaces)
    let withPort = trimmed.contains(":") ? trimmed : trimmed + ":4747"
    return URL(string: "http://" + withPort) ?? URL(string: "http://mac:4747")!
}
