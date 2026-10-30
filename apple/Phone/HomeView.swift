import SwiftUI
import WidgetKit

/// The space: its board's topics, newest first, and its devices.
struct HomeView: View {
    @Environment(Store.self) private var store
    @Environment(\.scenePhase) private var scenePhase
    @State private var path: [SidebarItem] = []
    @State private var settings = false

    var body: some View {
        NavigationStack(path: $path) {
            Group {
                if let space = store.space {
                    List {
                        Section {
                            NavigationLink(value: SidebarItem.board("")) { TopRow() }
                            ForEach(topics) { topic in
                                NavigationLink(value: SidebarItem.board(topic.path)) { TopicRow(topic: topic) }
                            }
                        }
                        Section {
                            NavigationLink(value: SidebarItem.devices) {
                                LabeledContent {
                                    Text("\(space.devices.filter(\.online).count) of \(space.devices.count) online")
                                } label: {
                                    Label("Devices", systemImage: "laptopcomputer.and.iphone")
                                }
                            }
                        }
                    }
                } else if case .unreachable(let message) = store.phase {
                    ContentUnavailableView {
                        Label("Can’t Reach Your Space", systemImage: "bolt.horizontal.circle")
                    } description: {
                        Text(message)
                    } actions: {
                        Text("It keeps trying.").foregroundStyle(.secondary)
                    }
                } else {
                    ProgressView()
                }
            }
            .navigationTitle(store.space?.title ?? "")
            .navigationBarTitleDisplayMode(.large)
            .refreshable { await refresh() }
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Settings", systemImage: "gearshape") { settings = true }
                }
            }
            .navigationDestination(for: SidebarItem.self) { item in
                Group {
                    switch item {
                    case .board(let topic): BoardView(topic: topic)
                    case .devices: DevicesView()
                    }
                }
                .navigationBarTitleDisplayMode(.inline)
                .refreshable { await refresh() }
                .onAppear { store.selection = item }
            }
        }
        .sheet(isPresented: $settings) { SettingsView() }
        .onChange(of: path) {
            store.selection = path.last ?? .board("")
            if path.isEmpty { Task { await store.refreshPage() } }
        }
        .onChange(of: store.recent) {
            guard let post = store.recent.first else { return }
            Newest(name: post.name, text: post.text, topic: post.topic, time: post.date, space: store.space?.title).save()
            WidgetCenter.shared.reloadAllTimelines()
        }
        .onChange(of: scenePhase) { if scenePhase == .active { Task { await refresh() } } }
    }

    /// Topics with posts, most recently active first, then the quiet ones by name.
    private var topics: [Topic] {
        store.topics.sorted { ($0.last ?? 0, $1.path) > ($1.last ?? 0, $0.path) }
    }

    private func refresh() async {
        try? await store.refreshState()
        await store.refreshBoard()
    }
}

private struct TopRow: View {
    @Environment(Store.self) private var store

    var body: some View {
        // At home the store's page is the top of the board; the newest posts elsewhere may not include it.
        let newest = store.page?.topic == "" ? store.page?.posts.last : store.recent.first { $0.topic.isEmpty }
        Row(symbol: "text.bubble.fill", accent: true, title: "Top of the Board", time: newest?.date,
            detail: newest.map(\.line) ?? String(localized: "For everyone in the space"))
    }
}

private struct TopicRow: View {
    @Environment(Store.self) private var store
    var topic: Topic

    /// Its newest post if it's among the board's newest, else what it's about.
    var body: some View {
        Row(symbol: "number", accent: false, title: topic.parent.isEmpty ? topic.name : "\(topic.parent)/\(topic.name)",
            time: topic.last.map { Date(timeIntervalSince1970: TimeInterval($0) / 1000) },
            detail: store.recent.first { $0.topic == topic.path }?.line ?? topic.description)
    }
}

private extension Post {
    var line: String { "\(displayName(name)): \(text)" }
}

/// A row as in Messages: a mark, a title with the time of its newest post, and a line or two under it.
private struct Row: View {
    var symbol: String
    var accent: Bool
    var title: String
    var time: Date?
    var detail: String

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: symbol)
                .font(.system(size: 16, weight: .semibold))
                .foregroundStyle(accent ? AnyShapeStyle(.white) : AnyShapeStyle(.secondary))
                .frame(width: 36, height: 36)
                .background(accent ? AnyShapeStyle(.tint) : AnyShapeStyle(.fill.tertiary), in: .circle)
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline) {
                    Text(title)
                        .font(.body.weight(.semibold))
                        .lineLimit(1)
                    Spacer(minLength: 8)
                    if let time {
                        TimelineView(.everyMinute) { _ in
                            Text(time.relative)
                                .font(.subheadline)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
                Text(detail)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }
        }
        .padding(.vertical, 2)
    }
}
