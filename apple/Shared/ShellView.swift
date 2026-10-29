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

// Shell is the app's view of the user's network. Every node holds the whole
// network and runs any plugin's operations wherever they live, so the app
// talks to whichever node answers first: this device's, or one of the
// devices it has seen.
@MainActor
final class Shell: ObservableObject {
    @Published var version: String?
    @Published var plugins: [Plugin] = []
    @Published var devices: (online: Int, total: Int)?
    @Published var error: String?
    @Published var widgets: [LiveWidget] = []
    @Published var pages: [AppPage] = []
    private(set) var client: MCPClient?
    private let preferred: [URL]

    init(nodes preferred: [URL]) {
        self.preferred = preferred
    }

    // The devices seen running a node, remembered for next time.
    private var known: [String] {
        get { UserDefaults.standard.stringArray(forKey: "nodes") ?? [] }
        set { UserDefaults.standard.set(newValue, forKey: "nodes") }
    }

    private func candidates() -> [URL] {
        var urls = client.map { [$0.node] } ?? []
        for url in preferred + known.compactMap({ URL(string: "http://\($0):4747") }) where !urls.contains(url) {
            urls.append(url)
        }
        return urls
    }

    private func healthy(_ node: URL) async -> String? {
        let req = URLRequest(url: node.appendingPathComponent("healthz"), timeoutInterval: 3)
        guard let (data, resp) = try? await URLSession.shared.data(for: req), (resp as? HTTPURLResponse)?.statusCode == 200 else { return nil }
        return ((try? JSONSerialization.jsonObject(with: data)) as? [String: Any])?["version"] as? String ?? ""
    }

    private func call(_ tool: String) async throws -> [String: Any] {
        guard let client else { throw MCPError.badResponse }
        let result = try await client.request("tools/call", ["name": tool, "arguments": [:] as [String: Any]])
        return result["structuredContent"] as? [String: Any] ?? [:]
    }

    func load() async {
        var reached: (URL, String)?
        for node in candidates() {
            if let v = await healthy(node) { reached = (node, v); break }
        }
        guard let (node, v) = reached else {
            error = "None of your devices answered. Is Tailscale on?"
            return
        }
        if client?.node != node { client = MCPClient(node: node, name: "The One App") }
        version = v
        do {
            let rows = try await call("plugins_list")["plugins"] as? [[String: Any]] ?? []
            plugins = rows.compactMap { r in
                guard let id = r["id"] as? String else { return nil }
                return Plugin(id: id, name: r["name"] as? String ?? id, version: r["version"] as? String,
                              enabled: r["enabled"] as? Bool ?? false, running: r["running"] as? Bool ?? false)
            }
            await loadWidgets()
            let pageRows = (try? await call("pages"))?["pages"] as? [[String: Any]] ?? []
            pages = pageRows.compactMap(AppPage.init)
            let list = try await call("devices")["devices"] as? [[String: Any]] ?? []
            devices = (list.filter { $0["online"] as? Bool == true }.count, list.count)
            known = list.compactMap { $0["name"] as? String }
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }
}

extension Shell {
    // Every tool marked as a widget, each drawn afresh on every load.
    func loadWidgets() async {
        guard let client,
              let tools = try? await client.request("tools/list")["tools"] as? [[String: Any]] else { return }
        let names = tools.compactMap { t -> String? in
            ((t["_meta"] as? [String: Any])?["dev.theoneapp/widget"] as? Bool) == true ? t["name"] as? String : nil
        }
        widgets = names.map { name in widgets.first { $0.tool == name } ?? LiveWidget(tool: name, client: client) }
        await withTaskGroup(of: Void.self) { group in
            for w in widgets { group.addTask { await w.load() } }
        }
    }
}

enum Page: Hashable, RawRepresentable {
    case home, plugins
    case page(String)

    init?(rawValue: String) {
        switch rawValue {
        case "home": self = .home
        case "plugins": self = .plugins
        default:
            guard rawValue.hasPrefix("page:") else { return nil }
            self = .page(String(rawValue.dropFirst(5)))
        }
    }

    var rawValue: String {
        switch self {
        case .home: "home"
        case .plugins: "plugins"
        case .page(let id): "page:" + id
        }
    }
}

// HomePage shows every widget on the user's devices.
struct HomePage: View {
    @ObservedObject var shell: Shell

    var body: some View {
        ScrollView {
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 300, maximum: 420), spacing: 16, alignment: .top)], alignment: .leading, spacing: 16) {
                ForEach(shell.widgets) { widget in
                    WidgetCard(widget: widget)
                }
            }
            .padding()
        }
        .overlay {
            if shell.widgets.isEmpty, let error = shell.error {
                ContentUnavailableView("Can't reach your devices", systemImage: "wifi.exclamationmark", description: Text(error))
            }
        }
        .refreshable { await shell.load() }
        .navigationTitle("Home")
    }
}

struct WidgetCard: View {
    @ObservedObject var widget: LiveWidget

    var body: some View {
        LiveWidgetView(widget: widget)
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .topLeading)
            .background(.background.secondary, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
    }
}

// ShellView is the app: a sidebar of pages, for plugins to add their own to
// later, beside the open page. On an iPhone it collapses to one column.
struct ShellView: View {
    @StateObject var shell: Shell
    // Remembered, so the app opens where it was left, as after an update.
    @AppStorage("page") private var page: Page = .home

    var body: some View {
        NavigationSplitView {
            List(selection: Binding<Page?>(get: { page }, set: { if let p = $0 { page = p } })) {
                SwiftUI.Section { identity }
                SwiftUI.Section {
                    NavigationLink(value: Page.home) {
                        Label("Home", systemImage: "square.grid.2x2")
                    }
                    NavigationLink(value: Page.plugins) {
                        Label("Plugins", systemImage: "puzzlepiece.extension")
                    }
                    ForEach(shell.pages) { p in
                        NavigationLink(value: Page.page(p.id)) {
                            Label(p.title, systemImage: p.icon)
                        }
                    }
                }
            }
            .navigationTitle("The One App")
            #if os(macOS)
            .navigationSplitViewColumnWidth(min: 200, ideal: 220)
            #endif
        } detail: {
            Group {
                switch page {
                case .home: HomePage(shell: shell)
                case .plugins: PluginsPage(shell: shell)
                case .page(let id):
                    if let p = shell.pages.first(where: { $0.id == id }), let client = shell.client {
                        PluginPage(page: p, client: client).id(p.id)
                    }
                }
            }
            .toolbar {
                ToolbarItem {
                    Button("Refresh", systemImage: "arrow.clockwise") { Task { await shell.load() } }
                }
            }
        }
        .task { await shell.load() }
    }

    // The title names the app, so this row shows the network it reaches.
    private var identity: some View {
        HStack(spacing: 10) {
            logo.resizable().frame(width: 36, height: 36).clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
            VStack(alignment: .leading, spacing: 1) {
                Text(devicesText).font(.headline)
                Text(shell.version ?? "").font(.caption).foregroundStyle(.secondary).lineLimit(1)
            }
        }
        .padding(.vertical, 2)
    }

    private var devicesText: String {
        guard let devices = shell.devices else { return shell.error == nil ? "Connecting…" : "Offline" }
        return devices.online == devices.total ? "\(devices.total) devices" : "\(devices.online) of \(devices.total) devices online"
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
                    ContentUnavailableView("Can't reach your devices", systemImage: "wifi.exclamationmark", description: Text(error))
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
