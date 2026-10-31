import Foundation
import Observation

/// Where a shell's navigation goes: an app's screen, or one of the shell's own views.
enum Route: Hashable {
    case screen(ScreenRoute)
    case devices
}

struct ScreenRoute: Hashable {
    var app: String
    var screen = "main"
    /// The screen's params, as JSON.
    var params = "{}"
}

enum Sheet: Hashable, Identifiable {
    case pair
    case join

    var id: Self { self }
}

extension Notification.Name {
    /// Each node event, as JSON in `object`, for the app screens that are open.
    static let nodeEvent = Notification.Name("nodeEvent")
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
    private(set) var apps: [AppInfo] = []
    private(set) var widgets: [WidgetInfo] = []
    /// The Mac's sidebar selection: the screen or view at the bottom of `path`.
    var selection: Route? {
        didSet { if selection != oldValue { path = [] } }
    }
    /// What's been pushed on top of it, by app screens or the shell.
    var path: [Route] = []
    var sheet: Sheet?

    init(node: NodeClient) {
        self.node = node
    }

    var brand: Brand? { state?.brand }
    /// The app's one space, once this device creates or joins it.
    var space: Space? { state?.space }
    /// The apps with screens, which shells list.
    var screenApps: [AppInfo] { apps.filter(\.hasScreens) }

    func app(_ name: String) -> AppInfo? { apps.first { $0.name == name } }

    /// An app's first screen.
    func main(_ app: AppInfo) -> Route { .screen(ScreenRoute(app: app.name)) }

    // MARK: Connection

    func run() async {
        while !Task.isCancelled {
            if await connect() {
                do {
                    for try await event in node.events() { await handle(event) }
                } catch {}
            }
            try? await Task.sleep(for: .seconds(phase == .ready ? 1 : 3))
        }
    }

    /// Whether the node answered.
    private func connect() async -> Bool {
        let failure: Error
        do {
            try await refreshState()
            return true
        } catch {
            failure = error
        }
        guard let startNode else {
            phase = .unreachable(failure.localizedDescription)
            return false
        }
        if phase != .ready { phase = .connecting }
        do {
            try await startNode()
        } catch {
            phase = .unreachable(error.localizedDescription)
        }
        for _ in 0..<40 {
            if (try? await refreshState()) != nil { return true }
            try? await Task.sleep(for: .milliseconds(500))
        }
        if case .unreachable = phase { return false }
        phase = .unreachable(node.transport.unreachable)
        return false
    }

    func refreshState() async throws {
        let state = try await node.state()
        let first = self.state == nil
        let arrived = self.state?.space?.member != true && state.space?.member == true
        self.state = state
        phase = .ready
        if first || arrived || apps.isEmpty {
            if first { await renameIfHostname() }
            await refreshSpace()
        }
    }

    private func handle(_ event: NodeEvent) async {
        if let json = try? JSONEncoder().encode(event) {
            NotificationCenter.default.post(name: .nodeEvent, object: String(decoding: json, as: UTF8.self))
        }
        if event.type == "changed" { await refreshWidgets() }
        if ["changed", "peers", "space"].contains(event.type) { try? await refreshState() }
    }

    private func renameIfHostname() async {
        guard let state, let name = preferredDeviceName, state.device.name.hasSuffix(".local"),
              !UserDefaults.standard.bool(forKey: "renamed") else { return }
        UserDefaults.standard.set(true, forKey: "renamed")
        try? await node.renameDevice(name)
        try? await refreshState()
    }

    // MARK: Apps

    /// The space's apps and widgets.
    func refreshSpace() async {
        if let apps = try? await node.apps() {
            self.apps = apps
            if selection == nil { selection = screenApps.first.map(main) ?? .devices }
        }
        await refreshWidgets()
    }

    func refreshWidgets() async {
        guard space?.member == true else { return }
        let widgets = try? await node.widgets()
        if let widgets, widgets != self.widgets { self.widgets = widgets }
    }

    // MARK: The space

    func createSpace(_ name: String) async throws {
        try await node.createSpace(name)
        try await refreshState()
    }

    func join(_ code: String) async throws {
        try await node.join(code.trimmingCharacters(in: .whitespacesAndNewlines))
        try await refreshState()
    }

    func renameDevice(_ name: String) async throws {
        try await node.renameDevice(name.trimmingCharacters(in: .whitespacesAndNewlines))
        try await refreshState()
    }

    func pair(newUser: Bool) async throws -> String {
        try await node.pair(newUser: newUser)
    }
}
