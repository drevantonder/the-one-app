import SwiftUI

struct Sidebar: View {
    @Environment(Store.self) private var store
    @State private var collapsed: Set<String> = []

    var body: some View {
        @Bindable var store = store
        List(selection: $store.selection) {
            Section {
                DisclosureGroup(isExpanded: expanded("")) {
                    TopicTree(parent: "", topics: store.topics, collapsed: $collapsed)
                } label: {
                    Label("Board", systemImage: "text.bubble")
                        .contextMenu { newTopic("") }
                }
                .tag(SidebarItem.board(""))
                Label("Devices", systemImage: "laptopcomputer.and.iphone")
                    .badge(store.space.map { "\($0.devices.filter(\.online).count)/\($0.devices.count)" } ?? "")
                    .tag(SidebarItem.devices)
            }
        }
        .safeAreaInset(edge: .top, spacing: 0) {
            SpaceSwitcher()
                .padding(.horizontal, 10)
                .padding(.bottom, 6)
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            Button {
                store.sheet = .newTopic(store.topic ?? "")
            } label: {
                Label("New Topic", systemImage: "plus.circle")
            }
            .buttonStyle(.borderless)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 18)
            .padding(.vertical, 12)
        }
    }

    private func expanded(_ path: String) -> Binding<Bool> {
        Binding { !collapsed.contains(path) } set: { open in
            if open { collapsed.remove(path) } else { collapsed.insert(path) }
        }
    }

    private func newTopic(_ parent: String) -> some View {
        Button(parent.isEmpty ? "New Topic…" : "New Topic in \(parent)…") { store.sheet = .newTopic(parent) }
    }
}

private struct TopicTree: View {
    @Environment(Store.self) private var store
    var parent: String
    var topics: [Topic]
    @Binding var collapsed: Set<String>

    var body: some View {
        ForEach(topics.filter { $0.parent == parent }) { topic in
            if topics.contains(where: { $0.parent == topic.path }) {
                DisclosureGroup(isExpanded: Binding { !collapsed.contains(topic.path) } set: { open in
                    if open { collapsed.remove(topic.path) } else { collapsed.insert(topic.path) }
                }) {
                    TopicTree(parent: topic.path, topics: topics, collapsed: $collapsed)
                } label: {
                    row(topic)
                }
                .tag(SidebarItem.board(topic.path))
            } else {
                row(topic).tag(SidebarItem.board(topic.path))
            }
        }
    }

    private func row(_ topic: Topic) -> some View {
        Label(topic.name, systemImage: "number")
            .help(topic.description)
            .contextMenu {
                Button("New Topic in \(topic.path)…") { store.sheet = .newTopic(topic.path) }
            }
    }
}

private struct SpaceSwitcher: View {
    @Environment(Store.self) private var store

    var body: some View {
        @Bindable var store = store
        Menu {
            Picker("Space", selection: $store.spaceID) {
                ForEach(store.spaces) { space in
                    Text(space.title).tag(Optional(space.id))
                }
            }
            .pickerStyle(.inline)
            .labelsHidden()
            Divider()
            Button("New Space…", systemImage: "plus") { store.sheet = .newSpace }
            Button("Join a Space…", systemImage: "ticket") { store.sheet = .join }
        } label: {
            HStack(spacing: 10) {
                SpaceMark(name: store.space?.title ?? "", size: 30)
                VStack(alignment: .leading, spacing: 1) {
                    Text(store.space?.title ?? "")
                        .font(.headline)
                        .lineLimit(1)
                    if let space = store.space {
                        Text(space.devices.count == 1 ? "1 device" : "\(space.devices.count) devices")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                Spacer(minLength: 4)
                Image(systemName: "chevron.up.chevron.down")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
            .contentShape(.rect(cornerRadius: 10))
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
    }
}

/// A space's mark: its initial on the brand accent.
struct SpaceMark: View {
    var name: String
    var size: CGFloat

    var body: some View {
        RoundedRectangle(cornerRadius: size * 0.27, style: .continuous)
            .fill(.tint)
            .overlay {
                Text(name.first.map { String($0).uppercased() } ?? "")
                    .font(.system(size: size * 0.5, weight: .semibold, design: .rounded))
                    .foregroundStyle(.white)
            }
            .frame(width: size, height: size)
    }
}
