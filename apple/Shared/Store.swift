import Foundation
import Observation

enum SidebarItem: Hashable {
    case board(String)
    case devices
}

enum Sheet: Hashable, Identifiable {
    case invite
    case join
    case newSpace
    case newTopic(String)

    var id: Self { self }
}

/// Everything the windows and the menu bar show, kept in step with the node's events.
@MainActor @Observable
final class Store {
    enum Phase: Equatable {
        case connecting
        case ready
        case unreachable(String)
    }

    let node: NodeClient
    /// Starts the node when it doesn't answer. Platform shells provide it.
    var startNode: (@MainActor () async throws -> Void)?
    /// A friendlier name for this device than its hostname, offered once.
    var preferredDeviceName: String?

    private(set) var phase = Phase.connecting
    private(set) var state: NodeState?
    private(set) var topics: [Topic] = []
    private(set) var page: BoardPage?
    private(set) var recent: [Post] = []
    var selection: SidebarItem? = .board("")
    var sheet: Sheet?
    var spaceID: String? {
        didSet {
            guard spaceID != oldValue else { return }
            UserDefaults.standard.set(spaceID, forKey: "space")
            topics = []; page = nil; recent = []
            selection = .board("")
            Task { await refreshBoard() }
        }
    }

    init(node: NodeClient = .local) {
        self.node = node
        spaceID = UserDefaults.standard.string(forKey: "space")
    }

    var brand: Brand? { state?.brand }
    var spaces: [Space] { state?.spaces ?? [] }
    var space: Space? { spaces.first { $0.id == spaceID } }
    var topic: String? { if case .board(let t) = selection { t } else { nil } }

    // MARK: Connection

    func run() async {
        while !Task.isCancelled {
            await connect()
            do {
                for try await event in node.events() { await handle(event) }
            } catch {}
            try? await Task.sleep(for: .seconds(1))
        }
    }

    private func connect() async {
        if (try? await refreshState()) != nil { return }
        if phase != .ready { phase = .connecting }
        do {
            try await startNode?()
        } catch {
            phase = .unreachable(error.localizedDescription)
        }
        for _ in 0..<40 {
            if (try? await refreshState()) != nil { return }
            try? await Task.sleep(for: .milliseconds(500))
        }
        if case .unreachable = phase { return }
        phase = .unreachable("The node isn't answering on \(node.base.host() ?? ""):\(node.base.port ?? 0).")
    }

    private func refreshState() async throws {
        let state = try await node.state()
        let first = self.state == nil
        self.state = state
        phase = .ready
        if space == nil { spaceID = state.spaces.first?.id }
        if first {
            await renameIfHostname()
            await refreshBoard()
        }
    }

    private func handle(_ event: NodeEvent) async {
        switch event.type {
        case "changed" where event.space == spaceID && (event.apps ?? []).contains("board"):
            await refreshBoard()
        case "changed", "peers", "spaces":
            try? await refreshState()
        default:
            break
        }
    }

    private func renameIfHostname() async {
        guard let state, let name = preferredDeviceName, state.device.name.hasSuffix(".local"),
              !UserDefaults.standard.bool(forKey: "renamed") else { return }
        UserDefaults.standard.set(true, forKey: "renamed")
        try? await node.renameDevice(name)
        try? await refreshState()
    }

    // MARK: Board

    func refreshBoard() async {
        guard let id = spaceID, space?.member == true else { return }
        async let page: Void = refreshPage()
        let topics: [Topic]? = try? await node.call("board", "topics", space: id)
        let recent = await latest(in: id, topics: topics ?? [])
        await page
        guard id == spaceID else { return }
        if let topics { self.topics = topics }
        self.recent = recent
    }

    /// The newest posts anywhere: the top of the board and the most recently active topics hold them.
    private func latest(in space: String, topics: [Topic]) async -> [Post] {
        let active = [""] + topics.filter { $0.last != nil }.sorted { $0.last! > $1.last! }.prefix(3).map(\.path)
        var posts: [Post] = []
        for topic in active {
            let page: BoardPage? = try? await node.call("board", "read", space: space, input: ["topic": .string(topic)])
            posts += page?.posts ?? []
        }
        return Array(posts.sorted { $0.time > $1.time }.prefix(6))
    }

    func refreshPage() async {
        guard let id = spaceID, let topic else { return }
        let page: BoardPage? = try? await node.call("board", "read", space: id, input: ["topic": .string(topic)])
        if id == spaceID, topic == self.topic { self.page = page }
    }

    func post(_ text: String) async throws {
        var input: [String: JSONValue] = ["text": .string(text)]
        if let topic, !topic.isEmpty { input["topic"] = .string(topic) }
        let _: String = try await node.call("board", "post", space: spaceID, input: input)
        await refreshBoard()
    }

    func createTopic(path: String, description: String) async throws {
        let _: String = try await node.call("board", "create", space: spaceID, input: ["path": .string(path), "description": .string(description)])
        await refreshBoard()
        selection = .board(path)
    }

    // MARK: Spaces

    func createSpace(_ name: String) async throws {
        let before = Set(spaces.map(\.id))
        try await node.createSpace(name)
        try await refreshState()
        if let new = spaces.first(where: { !before.contains($0.id) }) { spaceID = new.id }
    }

    func join(_ ticket: String) async throws {
        let before = Set(spaces.map(\.id))
        try await node.join(ticket.trimmingCharacters(in: .whitespacesAndNewlines))
        try await refreshState()
        if let new = spaces.first(where: { !before.contains($0.id) }) { spaceID = new.id }
    }

    func invite(mine: Bool) async throws -> String {
        guard let spaceID else { throw NodeError("Choose a space first") }
        return try await node.invite(space: spaceID, mine: mine)
    }
}
