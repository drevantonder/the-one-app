import SwiftUI

struct Plugin: Identifiable {
    let id: String
    let name: String
    let version: String?
    let enabled: Bool
    let running: Bool
}

@MainActor
final class Shell: ObservableObject {
    @Published var version: String?
    @Published var plugins: [Plugin] = []
    @Published var error: String?
    private(set) var client: MCPClient

    init(node: URL) {
        client = MCPClient(node: node, name: "The One App")
    }

    func connect(to node: URL) {
        client = MCPClient(node: node, name: "The One App")
        version = nil
        plugins = []
        Task { await load() }
    }

    func load() async {
        do {
            let (data, _) = try await URLSession.shared.data(from: client.node.appendingPathComponent("healthz"))
            version = ((try? JSONSerialization.jsonObject(with: data)) as? [String: Any])?["version"] as? String
            let result = try await client.request("tools/call", ["name": "plugins_list", "arguments": [:] as [String: Any]])
            let rows = (result["structuredContent"] as? [String: Any])?["plugins"] as? [[String: Any]] ?? []
            plugins = rows.compactMap { r in
                guard let id = r["id"] as? String else { return nil }
                return Plugin(id: id, name: r["name"] as? String ?? id, version: r["version"] as? String,
                              enabled: r["enabled"] as? Bool ?? false, running: r["running"] as? Bool ?? false)
            }
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }
}

// ShellView is the app's window: what the node runs, for plugins to add
// their own views to later.
struct ShellView: View {
    @StateObject var shell: Shell
    var settings: AnyView?

    var body: some View {
        List {
            SwiftUI.Section {
                HStack(spacing: 14) {
                    logo.resizable().frame(width: 56, height: 56).clipShape(RoundedRectangle(cornerRadius: 13, style: .continuous))
                    VStack(alignment: .leading, spacing: 2) {
                        Text("The One App").font(.title2.weight(.semibold))
                        Text(subtitle).font(.subheadline).foregroundStyle(.secondary)
                    }
                }
                .padding(.vertical, 4)
            }
            if let error = shell.error {
                SwiftUI.Section {
                    Label(error, systemImage: "wifi.exclamationmark").foregroundStyle(.secondary)
                }
            }
            SwiftUI.Section("Plugins") {
                ForEach(shell.plugins) { p in
                    HStack {
                        Text(String(p.name.prefix(1)))
                            .font(.headline).foregroundStyle(.white)
                            .frame(width: 30, height: 30)
                            .background(p.enabled ? Color.accentColor : Color.gray, in: RoundedRectangle(cornerRadius: 7, style: .continuous))
                        Text(p.name)
                        Spacer()
                        Text(p.version.map { "v\($0)" } ?? (p.enabled ? "Not on this device" : "Off"))
                            .font(.subheadline).foregroundStyle(.secondary)
                    }
                    .opacity(p.enabled ? 1 : 0.5)
                }
            }
        }
        .refreshable { await shell.load() }
        .task { await shell.load() }
        .toolbar {
            if let settings {
                ToolbarItem { settings }
            }
        }
    }

    private var subtitle: String {
        let host = shell.client.node.host ?? ""
        guard let version = shell.version else { return shell.error == nil ? "Connecting to \(host)…" : host }
        return "\(version) · \(host)"
    }

    private var logo: Image {
        #if os(macOS)
        Image(nsImage: NSApplication.shared.applicationIconImage)
        #else
        Image("Logo")
        #endif
    }
}
