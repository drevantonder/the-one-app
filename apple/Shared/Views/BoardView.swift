import SwiftUI

struct BoardView: View {
    @Environment(Store.self) private var store
    var topic: String

    private var page: BoardPage? { store.page?.topic == topic ? store.page : nil }

    var body: some View {
        Group {
            if let page {
                if page.posts.isEmpty {
                    ContentUnavailableView {
                        Label("Nothing Here Yet", systemImage: "text.bubble")
                    } description: {
                        Text(topic.isEmpty
                             ? "Posts at the top of the board are for everyone in the space. Write the first one below."
                             : "Posts in \(topic) will appear here as agents and people write them.")
                    }
                } else {
                    Feed(posts: page.posts)
                }
            } else {
                ProgressView().controlSize(.small)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .safeAreaInset(edge: .bottom, spacing: 0) {
            Composer(topic: topic)
        }
        .navigationTitle(topic.isEmpty ? "Board" : (topic.split(separator: "/").last.map(String.init) ?? topic))
        .navigationSubtitle(subtitle)
        .task(id: topic) { await store.refreshPage() }
    }

    private var subtitle: String {
        if let d = page?.description, !d.isEmpty { return d }
        return topic.isEmpty ? (store.space?.title ?? "") : topic
    }
}

private struct Feed: View {
    var posts: [Post]

    /// Consecutive posts by one author within ten minutes read as one group, as in Messages.
    struct Group: Identifiable {
        var posts: [Post]
        var newDay: Bool
        var id: String { posts[0].id }
        var first: Post { posts[0] }
    }

    var groups: [Group] {
        var out: [Group] = []
        for post in posts {
            if var last = out.last, last.first.person == post.person, last.first.name == post.name,
               post.time - last.posts.last!.time < 10 * 60_000,
               Calendar.current.isDate(post.date, inSameDayAs: last.first.date) {
                last.posts.append(post)
                out[out.count - 1] = last
            } else {
                let newDay = out.last.map { !Calendar.current.isDate(post.date, inSameDayAs: $0.first.date) } ?? true
                out.append(Group(posts: [post], newDay: newDay))
            }
        }
        return out
    }

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                ForEach(groups) { group in
                    if group.newDay { DayHeader(date: group.first.date) }
                    PostGroup(posts: group.posts)
                }
            }
            .frame(maxWidth: 720, alignment: .leading)
            .frame(maxWidth: .infinity)
            .padding(.horizontal, 28)
            .padding(.vertical, 12)
        }
        .defaultScrollAnchor(.bottom)
        .scrollEdgeEffectStyle(.soft, for: .bottom)
    }
}

private struct DayHeader: View {
    var date: Date

    var body: some View {
        Text(label)
            .font(.caption.weight(.medium))
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity)
            .padding(.top, 20)
            .padding(.bottom, 6)
    }

    private var label: String {
        if Calendar.current.isDateInToday(date) { return String(localized: "Today") }
        if Calendar.current.isDateInYesterday(date) { return String(localized: "Yesterday") }
        return date.formatted(.dateTime.weekday(.wide).day().month(.wide))
    }
}

private struct PostGroup: View {
    var posts: [Post]

    var body: some View {
        let first = posts[0]
        HStack(alignment: .top, spacing: 12) {
            Avatar(name: first.name, person: first.person, size: 34)
            VStack(alignment: .leading, spacing: 6) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(displayName(first.name))
                        .font(.body.weight(.semibold))
                    TimelineView(.everyMinute) { _ in
                        Text(first.date.relative)
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    }
                    .help(first.date.formatted(date: .complete, time: .shortened))
                }
                ForEach(posts) { post in
                    Text(post.text)
                        .font(.body)
                        .lineSpacing(2)
                        .textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                        .help(post.date.formatted(date: .abbreviated, time: .shortened))
                }
            }
        }
        .padding(.vertical, 9)
    }
}

func displayName(_ name: String) -> String {
    name.hasSuffix(".local") ? String(name.dropLast(6)) : name
}

private struct Composer: View {
    @Environment(Store.self) private var store
    var topic: String
    @State private var text = ""
    @State private var error: String?
    @State private var sending = false
    @FocusState private var focused: Bool

    private static let limit = 280
    private var bytes: Int { text.trimmingCharacters(in: .whitespacesAndNewlines).utf8.count }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let error {
                Label(error, systemImage: "exclamationmark.circle.fill")
                    .font(.callout)
                    .foregroundStyle(.red)
                    .padding(.horizontal, 6)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
            HStack(alignment: .bottom, spacing: 8) {
                TextField(topic.isEmpty ? "Post to the board" : "Post to \(topic)", text: $text, axis: .vertical)
                    .textFieldStyle(.plain)
                    .font(.body)
                    .lineLimit(1...6)
                    .focused($focused)
                    .onSubmit(send)
                    .padding(.vertical, 9)
                    .padding(.leading, 16)
                if bytes > Self.limit - 40 {
                    Text("\(Self.limit - bytes)")
                        .font(.callout.monospacedDigit())
                        .foregroundStyle(bytes > Self.limit ? .red : .secondary)
                        .padding(.bottom, 9)
                }
                Button(action: send) {
                    Image(systemName: "arrow.up.circle.fill")
                        .font(.system(size: 24))
                        .symbolRenderingMode(.hierarchical)
                }
                .buttonStyle(.plain)
                .foregroundStyle(bytes == 0 ? AnyShapeStyle(.tertiary) : AnyShapeStyle(.tint))
                .disabled(bytes == 0 || sending)
                .padding(.trailing, 6)
                .padding(.bottom, 4)
                .help("Post")
            }
            .glassEffect(.regular, in: .rect(cornerRadius: 20))
        }
        .frame(maxWidth: 720)
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 24)
        .padding(.top, 6)
        .padding(.bottom, 18)
        .animation(.snappy, value: error)
        .onChange(of: text) { if error != nil { error = nil } }
        .onChange(of: topic) { error = nil }
        .onAppear { focused = true }
    }

    private func send() {
        let line = text.replacingOccurrences(of: "\n", with: " ").trimmingCharacters(in: .whitespaces)
        guard !line.isEmpty, !sending else { return }
        sending = true
        Task {
            defer { sending = false }
            do {
                try await store.post(line)
                text = ""
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}
