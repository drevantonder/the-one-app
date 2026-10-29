import SwiftUI

struct Plugin: Identifiable {
    let id: String
    let name: String
    let version: String?
    let enabled: Bool
    let running: Bool

    // "declared" for a plugin added from an MCP server, which has no version.
    var shownVersion: String? {
        guard let version, version.first?.isNumber == true else { return nil }
        return "v" + version
    }

    var status: String {
        if !enabled { return "Off" }
        return running ? "Running" : "Not on this device"
    }
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

    var host: String { client.node.host ?? "" }

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

enum Page: Hashable {
    case plugins
}

// ShellView is the app: a sidebar of pages, for plugins to add their own to
// later, beside the open page. On an iPhone it collapses to one column.
struct ShellView: View {
    @StateObject var shell: Shell
    var settings: AnyView?
    @State private var page: Page? = .plugins

    var body: some View {
        NavigationSplitView {
            List(selection: $page) {
                NavigationLink(value: Page.plugins) {
                    Label("Plugins", systemImage: "puzzlepiece.extension")
                }
            }
            .safeAreaInset(edge: .top) { identity }
            .navigationTitle("The One App")
            #if os(macOS)
            .navigationSplitViewColumnWidth(min: 200, ideal: 220)
            #endif
        } detail: {
            switch page {
            case .plugins, nil:
                PluginsPage(shell: shell)
                    .toolbar {
                        ToolbarItem {
                            Button("Refresh", systemImage: "arrow.clockwise") { Task { await shell.load() } }
                        }
                        if let settings {
                            ToolbarItem { settings }
                        }
                    }
            }
        }
        .task { await shell.load() }
    }

    private var identity: some View {
        HStack(spacing: 10) {
            logo.resizable().frame(width: 36, height: 36).clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
            VStack(alignment: .leading, spacing: 1) {
                Text("The One App").font(.headline)
                Text(subtitle).font(.caption).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
    }

    private var subtitle: String {
        guard let version = shell.version else { return shell.error == nil ? "Connecting to \(shell.host)…" : shell.host }
        return "\(version) · \(shell.host)"
    }

    private var logo: Image {
        #if os(macOS)
        Image(nsImage: NSApplication.shared.applicationIconImage)
        #else
        Image("Logo")
        #endif
    }
}

struct PluginsPage: View {
    @ObservedObject var shell: Shell
    #if os(iOS)
    @Environment(\.horizontalSizeClass) private var sizeClass
    #endif

    var body: some View {
        content
            .overlay {
                if let error = shell.error, shell.plugins.isEmpty {
                    ContentUnavailableView("Can't reach \(shell.host)", systemImage: "wifi.exclamationmark", description: Text(error))
                }
            }
            .refreshable { await shell.load() }
            .navigationTitle("Plugins")
            #if os(macOS)
            .navigationSubtitle(shell.plugins.isEmpty ? "" : "\(shell.plugins.count) plugins")
            #endif
    }

    @ViewBuilder private var content: some View {
        #if os(iOS)
        if sizeClass == .compact { list } else { table }
        #else
        table
        #endif
    }

    private var table: some View {
        Table(shell.plugins) {
            TableColumn("Name") { p in
                HStack(spacing: 8) {
                    badge(p, size: 22)
                    Text(p.name)
                }
                .opacity(p.enabled ? 1 : 0.5)
            }
            TableColumn("Version") { p in
                Text(p.shownVersion ?? "—").foregroundStyle(.secondary).monospacedDigit()
            }
            .width(min: 70, ideal: 90)
            TableColumn("Status") { p in
                Text(p.status).foregroundStyle(p.running ? .primary : .secondary)
            }
            .width(min: 110, ideal: 150)
        }
    }

    private var list: some View {
        List(shell.plugins) { p in
            HStack {
                badge(p, size: 30)
                VStack(alignment: .leading, spacing: 1) {
                    Text(p.name)
                    Text(p.status).font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Text(p.shownVersion ?? "").font(.subheadline).foregroundStyle(.secondary)
            }
            .opacity(p.enabled ? 1 : 0.5)
        }
    }

    private func badge(_ p: Plugin, size: CGFloat) -> some View {
        Text(String(p.name.prefix(1)))
            .font(.system(size: size * 0.5, weight: .semibold)).foregroundStyle(.white)
            .frame(width: size, height: size)
            .background(p.enabled ? Color.accentColor : Color.gray, in: RoundedRectangle(cornerRadius: size * 0.24, style: .continuous))
    }
}
