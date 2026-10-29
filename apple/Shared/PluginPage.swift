import SwiftUI

// AppPage is a page a plugin adds to the sidebar: a list of items, each
// drawn as the widget its tool returns for it.
struct AppPage: Identifiable {
    let id: String
    let title: String
    let icon: String
    let tool: String
    let items: String? // none: the tool's widget is the whole page
    let settings: String?

    init?(_ row: [String: Any]) {
        guard let id = row["id"] as? String, let tool = row["tool"] as? String else { return nil }
        self.id = id
        self.title = row["title"] as? String ?? id
        self.icon = (row["icon"] as? String).flatMap { $0.isEmpty ? nil : $0 } ?? "doc.text"
        self.tool = tool
        self.items = (row["items"] as? String).flatMap { $0.isEmpty ? nil : $0 }
        self.settings = (row["settings"] as? String).flatMap { $0.isEmpty ? nil : $0 }
    }
}

struct PageItem: Identifiable {
    let id: String
    let title: String
    let depth: Int
}

// PluginPage lists a page's items beside the one chosen, like a mail
// client's mailboxes; on an iPhone, choosing one opens it.
struct PluginPage: View {
    let page: AppPage
    let client: MCPClient
    @State private var items: [PageItem] = []
    @AppStorage private var selection: String?
    @State private var error: String?
    #if os(iOS)
    @Environment(\.horizontalSizeClass) private var sizeClass
    #endif

    @State private var showSettings = false

    init(page: AppPage, client: MCPClient) {
        self.page = page
        self.client = client
        _selection = AppStorage("page.\(page.id).item")
    }

    var body: some View {
        Group {
            if page.items == nil {
                PageItemView(tool: page.tool, item: "", client: client, refresh: 5)
            } else {
                content.task {
                    while !Task.isCancelled {
                        await load()
                        try? await Task.sleep(for: .seconds(30))
                    }
                }
            }
        }
        .navigationTitle(page.title)
        .toolbar {
            if page.settings != nil {
                ToolbarItem { Button("Settings", systemImage: "gearshape") { showSettings = true } }
            }
        }
        .sheet(isPresented: $showSettings) {
            if let settings = page.settings {
                NavigationStack {
                    PageItemView(tool: settings, item: "", client: client)
                        .navigationTitle("\(page.title) settings")
                        .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { showSettings = false } } }
                }
                .frame(minWidth: 420, minHeight: 360)
            }
        }
    }

    @ViewBuilder private var content: some View {
        #if os(iOS)
        if sizeClass == .compact {
            List(items) { item in
                NavigationLink { PageItemView(tool: page.tool, item: item.id, client: client).navigationBarTitleDisplayMode(.inline) } label: { row(item) }
            }
            .overlay { empty }
            .refreshable { await load() }
        } else {
            split
        }
        #else
        split
        #endif
    }

    private var split: some View {
        HStack(spacing: 0) {
            List(items, selection: $selection) { item in row(item).tag(item.id) }
                .frame(width: 210)
            Divider()
            Group {
                if let item = items.first(where: { $0.id == selection }) {
                    PageItemView(tool: page.tool, item: item.id, client: client).id(item.id)
                } else {
                    empty
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    @ViewBuilder private var empty: some View {
        if items.isEmpty, let error {
            ContentUnavailableView("Can't load \(page.title)", systemImage: "exclamationmark.triangle", description: Text(error))
        }
    }

    private func row(_ item: PageItem) -> some View {
        HStack(spacing: 6) {
            Image(systemName: "number").foregroundStyle(.secondary).imageScale(.small)
            Text(item.title).lineLimit(1)
        }
        .padding(.leading, CGFloat(item.depth) * 14)
    }

    private func load() async {
        guard let items = page.items else { return }
        do {
            let result = try await client.request("tools/call", ["name": items, "arguments": [:] as [String: Any]])
            let rows = (result["structuredContent"] as? [String: Any])?["items"] as? [[String: Any]] ?? []
            self.items = rows.compactMap { r in
                guard let id = r["id"] as? String else { return nil }
                return PageItem(id: id, title: r["title"] as? String ?? id, depth: r["depth"] as? Int ?? 0)
            }
            if selection == nil || !self.items.contains(where: { $0.id == selection }) { selection = self.items.first?.id }
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }
}

// PageItemView draws one item's widget, kept fresh while it's open.
struct PageItemView: View {
    @StateObject private var widget: LiveWidget
    private let refresh: Double

    init(tool: String, item: String, client: MCPClient, refresh: Double = 15) {
        _widget = StateObject(wrappedValue: LiveWidget(tool: tool, client: client, input: ["item": item, "tz": TimeZone.current.identifier]))
        self.refresh = refresh
    }

    var body: some View {
        ScrollView {
            LiveWidgetView(widget: widget)
                .frame(maxWidth: 1100, alignment: .leading)
                .padding(20)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .refreshable { await widget.load() }
        .task {
            while !Task.isCancelled {
                await widget.load()
                try? await Task.sleep(for: .seconds(refresh))
            }
        }
    }
}
