import Foundation
import Observation

/// A page: one scope the user can read, showing the apps installed there and their widgets.
struct Page: Identifiable, Hashable {
    /// `user`, `group:<id>` or `global`: also the hint a screen opened from here gets as `one.scope`.
    var scope: String
    var title: String
    var subtitle: String

    var id: String { scope }
    var group: String? { scope.hasPrefix("group:") ? String(scope.dropFirst(6)) : nil }
}

/// Where a shell's navigation goes: a page, an app's screen, or one of the shell's own views.
enum Route: Hashable {
    case page(String)
    case screen(ScreenRoute)
    case settings
    case setting(Setting)
    case group(String)
}

struct ScreenRoute: Hashable {
    var app: String
    var screen = "main"
    /// The screen's params, as JSON.
    var params = "{}"
    /// The page it was opened from.
    var scope: String?
}

/// The settings page's icons.
enum Setting: String, Hashable, CaseIterable, Identifiable {
    case store, groups, devices, secrets, access, users, backups, requests

    var id: Self { self }

    var title: String {
        switch self {
        case .store: String(localized: "App Store")
        case .groups: String(localized: "Groups")
        case .devices: String(localized: "Devices")
        case .secrets: String(localized: "Secrets")
        case .access: String(localized: "Access Log")
        case .users: String(localized: "Users")
        case .backups: String(localized: "Backups")
        case .requests: String(localized: "Requests")
        }
    }

    var symbol: String {
        switch self {
        case .store: "bag.fill"
        case .groups: "person.3.fill"
        case .devices: "laptopcomputer.and.iphone"
        case .secrets: "key.fill"
        case .access: "eye.fill"
        case .users: "person.2.badge.gearshape.fill"
        case .backups: "externaldrive.fill.badge.timemachine"
        case .requests: "tray.full.fill"
        }
    }
}

enum Sheet: Hashable, Identifiable {
    case pair
    case join
    case pages

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
    /// A friendlier name for this machine than its hostname, offered once.
    var preferredDeviceName: String?

    private(set) var phase = Phase.connecting
    private(set) var state: NodeState?
    private(set) var apps: [AppInfo] = []
    /// Each page's widgets, rendered for its scope.
    private(set) var widgets: [String: [WidgetInfo]] = [:]
    /// The Mac's sidebar selection: the page, screen or view at the bottom of `path`.
    var selection: Route? {
        didSet { if selection != oldValue { path = [] } }
    }
    /// What's been pushed on top of it, by app screens or the shell.
    var path: [Route] = []
    /// The iPhone's current page: a scope, or `settings`.
    var current = "user"
    var sheet: Sheet?

    /// The order this user put their group pages in, and the ones they hid, on this machine.
    private(set) var pageOrder = UserDefaults.standard.stringArray(forKey: "pageOrder") ?? []
    private(set) var hiddenPages = Set(UserDefaults.standard.stringArray(forKey: "hiddenPages") ?? [])

    init(node: NodeClient) {
        self.node = node
    }

    var brand: Brand? { state?.brand }
    var install: Install? { state?.install }
    var me: Person? { state?.user }
    var isAdmin: Bool { me?.admin == true }

    func app(_ name: String) -> AppInfo? { apps.first { $0.name == name } }
    func userName(_ id: String) -> String {
        id == me?.id ? String(localized: "You") : install?.user(id)?.name ?? String(id.prefix(6))
    }

    // MARK: Pages

    /// Every page this user can read: theirs, their groups' in their order, then everyone's.
    var allPages: [Page] {
        guard let install, let me else { return [] }
        let mine = install.groups.filter { $0.role(of: me.id) != nil }
            .sorted { (pageOrder.firstIndex(of: "group:\($0.id)") ?? .max, $0.name) < (pageOrder.firstIndex(of: "group:\($1.id)") ?? .max, $1.name) }
        let devices = install.servers.filter { $0.user == me.id }.count + install.clients.filter { $0.user == me.id }.count
        var pages = [Page(scope: "user", title: String(localized: "You"),
                          subtitle: devices == 1 ? String(localized: "Just you · 1 device") : String(localized: "Just you · \(devices) devices"))]
        pages += mine.map { group in
            Page(scope: "group:\(group.id)", title: group.name, subtitle: "\(people(group.members.count)) · \(arrangedBy(group.admins))")
        }
        pages.append(Page(scope: "global", title: String(localized: "Everyone"),
                          subtitle: "\(people(install.users.count)) · \(arrangedBy(install.users.filter(\.admin).map(\.id)))"))
        return pages
    }

    /// The pages shown: hidden group pages left out.
    var pages: [Page] { allPages.filter { !hiddenPages.contains($0.scope) } }

    func page(_ scope: String) -> Page? { allPages.first { $0.scope == scope } }

    private func people(_ n: Int) -> String { n == 1 ? String(localized: "1 person") : String(localized: "\(n) people") }

    private func arrangedBy(_ admins: [String]) -> String {
        let names = admins.sorted { ($0 == me?.id ? 1 : 0, userName($0)) < ($1 == me?.id ? 1 : 0, userName($1)) }
            .map { $0 == me?.id ? String(localized: "you") : userName($0) }
        switch names.count {
        case 0: return String(localized: "arranged by admins")
        case 1: return String(localized: "arranged by \(names[0])")
        case 2: return String(localized: "arranged by \(names[0]) and \(names[1])")
        default: return String(localized: "arranged by \(names.count) admins")
        }
    }

    /// Whether this user may install into, and so arrange, a scope: their own, a group they admin, or everyone's as an admin.
    func canArrange(_ scope: String) -> Bool {
        if scope == "user" { return true }
        if scope == "global" { return isAdmin }
        guard let id = Page(scope: scope, title: "", subtitle: "").group, let me else { return false }
        return install?.group(id)?.role(of: me.id) == "admin"
    }

    /// The apps on a page: those installed in its scope that have a screen.
    func apps(on page: Page) -> [AppInfo] {
        apps.filter { $0.hasScreens && $0.scopes.contains(page.scope) }
    }

    /// A page's widgets: those of the apps installed in its scope, screen or not.
    func widgets(on page: Page) -> [WidgetInfo] {
        let here = Set(apps.filter { $0.scopes.contains(page.scope) }.map(\.name))
        return (widgets[page.scope] ?? []).filter { here.contains($0.app) }
    }

    /// Every page's widgets, once each: for the menu bar and the home-screen widget.
    var allWidgets: [WidgetInfo] {
        var seen = Set<String>()
        return pages.flatMap { widgets(on: $0) }.filter { seen.insert($0.id).inserted }
    }

    /// Opens an app's first screen, telling it which page it came from.
    func open(_ app: AppInfo, from page: Page?) -> Route {
        .screen(ScreenRoute(app: app.name, scope: page?.scope))
    }

    func movePages(from source: IndexSet, to destination: Int) {
        var groups = allPages.filter { $0.group != nil }.map(\.scope)
        groups.move(fromOffsets: source, toOffset: destination)
        pageOrder = groups
        UserDefaults.standard.set(pageOrder, forKey: "pageOrder")
    }

    func setHidden(_ scope: String, _ hidden: Bool) {
        if hidden { hiddenPages.insert(scope) } else { hiddenPages.remove(scope) }
        UserDefaults.standard.set(Array(hiddenPages), forKey: "hiddenPages")
        if hidden, case .page(scope) = selection { selection = .page("user") }
    }

    // MARK: Settings

    /// Icons by role: Users and Backups only for admins, Requests only while something waits.
    var settings: [Setting] {
        var items: [Setting] = [.store, .groups, .devices, .secrets, .access]
        if isAdmin { items += [.users, .backups] }
        if !invites.isEmpty || !trustOffers.isEmpty { items.insert(.requests, at: 0) }
        return items
    }

    /// Groups that invited this user.
    var invites: [GroupInfo] {
        guard let me else { return [] }
        return install?.groups.filter { $0.invited?.contains { $0.user == me.id } == true } ?? []
    }

    /// Servers this user may use, through a group or by name, but doesn't trust yet.
    var trustOffers: [Server] {
        guard let me, let install else { return [] }
        let mine = ["user:\(me.id)"] + install.groups.filter { $0.role(of: me.id) != nil }.map { "group:\($0.id)" }
        return install.servers.filter { s in
            !s.isPrivileged && !s.owners.contains("user:\(me.id)") && s.users.contains { mine.contains($0) }
                && s.trusted_by?.contains(me.id) != true
        }
    }

    var hasRequests: Bool { settings.contains(.requests) }

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
        let arrived = self.state?.install?.joining != false && state.install?.joining == false
        let regrouped = state.user != self.state?.user || state.install?.groups != self.state?.install?.groups
        self.state = state
        phase = .ready
        if first { await renameIfHostname() }
        if first || arrived || apps.isEmpty || regrouped { await refreshApps() }
    }

    private func handle(_ event: NodeEvent) async {
        if let json = try? JSONEncoder().encode(event) {
            NotificationCenter.default.post(name: .nodeEvent, object: String(decoding: json, as: UTF8.self))
        }
        if event.type == "changed" { await refreshWidgets() }
        if ["changed", "servers", "install", "peers", "space"].contains(event.type) { try? await refreshState() }
    }

    private func renameIfHostname() async {
        guard let state, let name = preferredDeviceName, state.machine.name.hasSuffix(".local"),
              !UserDefaults.standard.bool(forKey: "renamed") else { return }
        UserDefaults.standard.set(true, forKey: "renamed")
        try? await node.rename(name)
        try? await refreshState()
    }

    // MARK: Apps

    func refreshApps() async {
        guard install != nil else { return }
        if let apps = try? await node.apps(), apps != self.apps { self.apps = apps }
        if selection == nil { selection = .page("user") }
        await refreshWidgets()
    }

    func refreshWidgets() async {
        guard install?.joining == false else { return }
        let node = node
        var fresh: [String: [WidgetInfo]] = [:]
        await withTaskGroup(of: (String, [WidgetInfo]?).self) { tasks in
            for page in allPages { tasks.addTask { (page.scope, try? await node.widgets(scope: page.scope)) } }
            for await (scope, widgets) in tasks { fresh[scope] = widgets ?? self.widgets[scope] ?? [] }
        }
        if fresh != widgets { widgets = fresh }
    }

    // MARK: Changes

    /// Runs a change, then takes in what it changed.
    func change(_ work: @MainActor (NodeClient) async throws -> Void) async throws {
        try await work(node)
        try await refreshState()
        await refreshApps()
    }

    func join(_ code: String) async throws {
        try await change { try await $0.join(code.trimmingCharacters(in: .whitespacesAndNewlines)) }
    }

    func rename(_ name: String) async throws {
        try await change { try await $0.rename(name.trimmingCharacters(in: .whitespacesAndNewlines)) }
    }
}
