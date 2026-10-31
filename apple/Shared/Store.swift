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
enum Route: Hashable, Codable {
    case page(String)
    case screen(ScreenRoute)
    case settings
    case setting(Setting)
    case group(String)
    /// A page's Connected Apps folder, opened: its scope.
    case connected(String)
}

struct ScreenRoute: Hashable, Codable {
    var app: String
    var screen = "main"
    /// The screen's params, as JSON.
    var params = "{}"
    /// The page it was opened from.
    var scope: String?
}

/// The settings page's icons.
enum Setting: String, Hashable, Codable, CaseIterable, Identifiable {
    case store, groups, devices, secrets, access, users, install, backups, requests, appearance

    var id: Self { self }

    var title: String {
        switch self {
        case .store: String(localized: "App Store")
        case .groups: String(localized: "Groups")
        case .devices: String(localized: "Devices")
        case .secrets: String(localized: "Secrets")
        case .access: String(localized: "Access Log")
        case .users: String(localized: "Users")
        case .install: String(localized: "Install")
        case .backups: String(localized: "Backups")
        case .requests: String(localized: "Requests")
        case .appearance: String(localized: "Appearance")
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
        case .install: "slider.horizontal.3"
        case .backups: "externaldrive.fill.badge.timemachine"
        case .requests: "tray.full.fill"
        case .appearance: "circle.lefthalf.filled"
        }
    }
}

enum Sheet: Hashable, Identifiable {
    case pair
    case join
    case pages
    /// A request that this user set a secret, from a `<cli>://secret?id=…&scope=…` link.
    case secret(scope: String, id: String)
    /// A page's widgets, to show, hide and size, with every other widget there is.
    case widgets(String)
    /// A page's Connected Apps folder, on a Mac: its scope.
    case connected(String)

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
    /// Each page's apps' badges, by scope then app.
    private(set) var badges: [String: [String: Int]] = [:]
    /// The Mac's sidebar selection: the page, screen or view at the bottom of `path`. It's kept, so the app opens
    /// where it was, after an update too.
    var selection: Route? {
        didSet {
            guard selection != oldValue else { return }
            path = []
            UserDefaults.standard.set(selection.flatMap { try? JSONEncoder().encode($0) }, forKey: "selection")
        }
    }
    /// The Mac's sidebar, folded to icons (⌘\).
    var sidebarFolded = UserDefaults.standard.bool(forKey: "sidebarFolded") {
        didSet { UserDefaults.standard.set(sidebarFolded, forKey: "sidebarFolded") }
    }
    /// What's been pushed on top of it, by app screens or the shell.
    var path: [Route] = []
    /// The iPhone's current page: a scope.
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

    /// A page's connected apps, which its Connected Apps folder holds instead of each having an icon.
    func connectedApps(on page: Page) -> [AppInfo] {
        apps.filter { $0.isConnected && $0.scopes.contains(page.scope) }
            .sorted { $0.label.localizedStandardCompare($1.label) == .orderedAscending }
    }

    /// Whether this user acts for an app's owner, and so may delete it: it's theirs, their group's as its admin, or
    /// the admins' and they're one.
    func owns(_ app: AppInfo) -> Bool {
        guard let owner = app.owner, let me else { return false }
        if owner == "admins" { return isAdmin }
        if owner == "user:\(me.id)" { return true }
        return owner.hasPrefix("group:") && install?.group(String(owner.dropFirst(6)))?.role(of: me.id) == "admin"
    }

    /// The App Store's page for an app, if the brand's store app is here and the app is one it lists (drivers aren't).
    func storeRoute(for app: AppInfo) -> Route? {
        guard let name = brand?.store, let store = self.app(name), store.has(screen: "app"), (app.deploys ?? 0) > 0 else { return nil }
        return .screen(ScreenRoute(app: name, screen: "app", params: Self.params(["app": app.name])))
    }

    /// The App Store, showing only connected apps, if the brand's store app is here.
    var connectedStoreRoute: Route? {
        guard let name = brand?.store, app(name)?.hasScreens == true else { return nil }
        return .screen(ScreenRoute(app: name, params: Self.params(["connected": true])))
    }

    /// An app's settings screen, opened from a page; with `add`, starting to sign in another account.
    func settingsRoute(for app: AppInfo, on page: Page, add: Bool = false) -> Route? {
        guard app.has(screen: "settings") else { return nil }
        return .screen(ScreenRoute(app: app.name, screen: "settings", params: Self.params(add ? ["add": true] : [:]), scope: page.scope))
    }

    static func params(_ fields: [String: Any]) -> String {
        (try? JSONSerialization.data(withJSONObject: fields, options: .sortedKeys)).map { String(decoding: $0, as: UTF8.self) } ?? "{}"
    }

    /// The count an app's icon shows on a page, if any.
    func appBadge(_ app: String, on page: Page) -> Int? {
        badges[page.scope]?[app].flatMap { $0 > 0 ? $0 : nil }
    }

    /// A page's widgets: those of the apps installed in its scope, screen or not, but for those hidden from it.
    func widgets(on page: Page, hidden: Bool = false) -> [WidgetInfo] {
        let here = Set(apps.filter { $0.scopes.contains(page.scope) }.map(\.name))
        return (widgets[page.scope] ?? []).filter { here.contains($0.app) && (hidden || !hiddenWidgets.contains("\(page.scope)/\($0.id)")) }
    }

    // MARK: Widgets

    /// The widgets someone hid from a page, and the sizes they gave others, on this machine, as `<scope>/<app>.<widget>`.
    private(set) var hiddenWidgets = Set(UserDefaults.standard.stringArray(forKey: "pageHiddenWidgets") ?? [])
    private(set) var widgetSizes = UserDefaults.standard.dictionary(forKey: "widgetSizes") as? [String: String] ?? [:]

    /// The size someone gave a widget on a page, if it still draws that size; else its own.
    func size(of widget: WidgetInfo, on page: Page) -> String {
        let chosen = widgetSizes["\(page.scope)/\(widget.id)"]
        return chosen.flatMap { widget.supported.contains($0) ? $0 : nil } ?? widget.size ?? "medium"
    }

    func setSize(_ size: String, of widget: WidgetInfo, on page: Page) {
        widgetSizes["\(page.scope)/\(widget.id)"] = size
        UserDefaults.standard.set(widgetSizes, forKey: "widgetSizes")
    }

    func setHidden(_ widget: WidgetInfo, on page: Page, _ hidden: Bool) {
        let id = "\(page.scope)/\(widget.id)"
        if hidden { hiddenWidgets.insert(id) } else { hiddenWidgets.remove(id) }
        UserDefaults.standard.set(Array(hiddenWidgets), forKey: "pageHiddenWidgets")
    }

    /// Runs an operation a widget's button or toggle names, as this user, then draws the widgets again. The error's
    /// text, if it failed.
    func run(app: String, op: String, input: JSONValue) async -> String? {
        do {
            _ = try await node.call(app: app, op: op, input: input.data)
        } catch {
            return error.localizedDescription
        }
        await refreshWidgets()
        return nil
    }

    /// Where a widget's link goes, if it's one of the app's screens.
    func route(for link: WidgetLink, app: String, scope: String?) -> Route? {
        guard case .screen(let screen, let params) = link else { return nil }
        return .screen(ScreenRoute(app: app, screen: screen, params: String(decoding: (params ?? .object([:])).data, as: UTF8.self), scope: scope))
    }

    /// Draws the widgets again when the soonest of them asks to be (`next`), as well as on every change.
    private var redraw: Task<Void, Never>?

    private func scheduleRedraw() {
        redraw?.cancel()
        guard let soonest = widgets.values.flatMap({ $0 }).compactMap({ $0.root.redraw() }).min() else { return }
        redraw = Task {
            try? await Task.sleep(for: .seconds(soonest.timeIntervalSinceNow))
            if !Task.isCancelled { await refreshWidgets() }
        }
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

    /// Icons by role: Users, Install and Backups only for admins, Requests first, only while something waits.
    var settings: [Setting] {
        var items: [Setting] = [.store, .groups, .devices, .secrets, .access]
        if isAdmin { items += [.users, .install, .backups] }
        if requests > 0 { items.insert(.requests, at: 0) }
        return items
    }

    /// What waits for this user: invites, servers to trust and secrets asked for.
    var requests: Int { invites.count + trustOffers.count + (state?.secretRequests ?? 0) }

    /// The count on Settings' icon: the requests waiting.
    var badge: Int { requests }

    /// The count on the app's own icon, as iOS adds up a folder's: every app's badge on every page, and the requests.
    var iconBadge: Int { requests + badges.values.flatMap(\.values).reduce(0) { $0 + max($1, 0) } }

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

    var hasRequests: Bool { badge > 0 }

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
        if ["servers", "install"].contains(event.type) { try? await refreshState() }
        // Installs, deploys and data all arrive as changes; the node's own entries (a secret request) change no app.
        if event.type == "changed" {
            let apps = event.apps ?? []
            if apps.isEmpty { try? await refreshState() }
            await refreshApps()
        }
    }

    /// `<cli>://secret?id=…&scope=…`, which an agent gives a person to set a secret it asked for. Whether it was one.
    @discardableResult
    func open(_ url: URL) -> Bool {
        let query = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems ?? []
        let value = { (name: String) in query.first { $0.name == name }?.value ?? "" }
        guard url.host() == "secret", !value("id").isEmpty, !value("scope").isEmpty else { return false }
        sheet = .secret(scope: value("scope"), id: value("id"))
        return true
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
        do {
            let apps = try await node.apps()
            if apps != self.apps { self.apps = apps }
            // A node that just started lists its deployed apps before it can describe them.
            if apps.contains(where: { ($0.deploys ?? 0) > 0 && $0.title == nil }) { throw NodeError("starting") }
        } catch {
            Task {
                try? await Task.sleep(for: .seconds(2))
                await refreshApps()
            }
        }
        if selection == nil {
            let kept = UserDefaults.standard.data(forKey: "selection").flatMap { try? JSONDecoder().decode(Route.self, from: $0) }
            selection = kept.flatMap(stillThere) ?? .page("user")
        }
        await refreshWidgets()
    }

    func refreshWidgets() async {
        guard let install, !install.joining else { return }
        let node = node
        var fresh: [String: [WidgetInfo]] = [:]
        await withTaskGroup(of: (String, [WidgetInfo]?).self) { tasks in
            for page in allPages { tasks.addTask { (page.scope, try? await node.widgets(scope: page.scope)) } }
            for await (scope, widgets) in tasks { fresh[scope] = widgets ?? self.widgets[scope] ?? [] }
        }
        if fresh != widgets { widgets = fresh }
        scheduleRedraw()
        var counts: [String: [String: Int]] = [:]
        for page in allPages { counts[page.scope] = (try? await node.badges(scope: page.scope)) ?? [:] }
        if counts != badges { badges = counts }
    }

    /// A route kept from before, if it still leads somewhere.
    private func stillThere(_ route: Route) -> Route? {
        switch route {
        case .page(let scope): pages.contains { $0.scope == scope } ? route : nil
        // A screen the app no longer has (Gmail's main, once it was a connected app) isn't; one not described yet may be.
        case .screen(let screen): app(screen.app).map { $0.screens == nil || $0.has(screen: screen.screen) } == true ? route : nil
        case .group(let id): install?.group(id) != nil ? route : nil
        case .connected(let scope): page(scope) != nil ? route : nil
        case .settings, .setting: route
        }
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
