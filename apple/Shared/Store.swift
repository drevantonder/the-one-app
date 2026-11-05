import Foundation
import Observation
import WebKit
#if os(macOS)
import AppKit
#endif

/// A page: one scope the user can read, showing the apps installed there and their widgets.
struct Page: Identifiable, Hashable {
    /// `user`, `group:<id>` or `global`: also the hint a screen opened from here gets as `one.scope`.
    var scope: String
    var title: String

    var id: String { scope }
}

/// Where a shell's navigation goes: a page, an app's screen, Settings, one of its screens, or a page's Services.
enum Route: Hashable, Codable {
    case page(String)
    case screen(ScreenRoute)
    case settings
    /// A Settings screen, by its item's name, with its params as JSON.
    case setting(String, params: String = "{}")
    /// A page's Services folder, opened: its scope.
    case services(String)
}

struct ScreenRoute: Hashable, Codable {
    var app: String
    var screen = "main"
    /// The screen's params, as JSON.
    var params = "{}"
    /// The page it was opened from.
    var scope: String?
}

enum Sheet: Hashable, Identifiable {
    case pair
    case join
    /// A request for a Vault item, from a `<cli>://vault?id=…&scope=…` link, or an app screen's `one.vault(need)`.
    case vault(scope: String, id: String)
    /// The shell's own request for an item, made here rather than as an entry (push's Apple key, from Settings).
    case ask(VaultRequest)

    var id: Self { self }
}

#if os(iOS)
/// A screen slid up over the others (the bridge's `open` with `sheet`), and what it gave `one.close`.
@MainActor
final class ScreenSheet: Identifiable {
    let id = UUID()
    let route: Route
    var value: Any?

    init(route: Route) { self.route = route }
}
#endif

extension Notification.Name {
    /// Each node event, as JSON in `object`, for the app screens that are open.
    static let nodeEvent = Notification.Name("nodeEvent")
}

/// Everything the windows and the menu bar show, kept in step with the node's events.
@MainActor @Observable
final class Store {
    enum Phase: Equatable {
        case connecting
        /// Waiting to be approved by one of the person's devices, showing this code.
        case joining(String)
        case ready
        case unreachable(String)
    }

    let node: NodeClient
    /// Starts the node when it doesn't answer. Platform shells provide it.
    var startNode: (@MainActor () async throws -> Void)?
    /// A friendlier name for this machine than its hostname, offered once.
    var preferredDeviceName: String?
    /// Whether someone is signed in, so their apps and widgets are asked for: until they are, a Mac's window asks its
    /// node only what it is.
    var signedIn = true

    private(set) var phase = Phase.connecting
    private(set) var state: NodeState?
    private(set) var apps: [AppInfo] = []
    /// Each page's widgets, rendered for its scope.
    private(set) var widgets: [String: [WidgetInfo]] = [:]
    /// When every page's widgets last came from the node.
    private(set) var widgetsFetched: Date?
    /// Each page's apps' badges, by scope then app.
    private(set) var badges: [String: [String: Int]] = [:]
    /// The Mac's sidebar selection: the page, screen or view at the bottom of `path`. It's kept, so the app opens
    /// where it was, after an update too.
    var selection: Route? {
        didSet {
            guard selection != oldValue else { return }
            // A Settings item with a url (Passkeys) opens in the browser, and the sidebar stays where it was.
            if case .setting(let name, _) = selection, let url = settingsItem(name)?.url {
                #if os(macOS)
                NSWorkspace.shared.open(url)
                #endif
                selection = oldValue
                return
            }
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
    /// Developer mode, on this device (Settings › Developer): the bar's </> shows only in it.
    var developer = false
    /// The Mac's Developer panel, docked under the screen in front: it stays open from screen to screen until closed.
    var devtoolsOpen = UserDefaults.standard.bool(forKey: "devtoolsOpen") {
        didSet { UserDefaults.standard.set(devtoolsOpen, forKey: "devtoolsOpen") }
    }
    /// How tall that panel is once its top edge was dragged, kept for next time and used on every screen; until then it has its own.
    var devtoolsHeight = UserDefaults.standard.object(forKey: "devtoolsHeight") as? Double {
        didSet { UserDefaults.standard.set(devtoolsHeight, forKey: "devtoolsHeight") }
    }
    #if os(iOS)
    /// Screens slid up over the others, by the bridge's `open` with `sheet`, each over the one before.
    var screenSheets: [ScreenSheet] = []
    #endif

    init(node: NodeClient) {
        self.node = node
    }

    /// Counted up as the person signs out (`forget`): what began before it writes nothing of theirs after, here or in
    /// the window kept for next time.
    private var session = 0

    /// Throws once the person who was signed in at `began` signed out.
    private func current(_ began: Int) throws {
        if began != session { throw CancellationError() }
    }

    /// The node's answer, kept as the window's (`KeptWindow`) to draw the next time the app opens, if its person is
    /// still signed in.
    private func kept<T>(_ name: String, _ answer: (T, Data), since began: Int) throws -> T {
        try current(began)
        KeptWindow.keep(name, answer.1)
        return answer.0
    }

    /// Forgets everything it showed, and what it kept of the person's on this device: they signed out.
    func forget() {
        session += 1
        KeptWindow.forget()
        answered = false
        ScreenCache.forget()
        forgetKeptScreens()
        ScreenFile.forget()
        URLCache.shared.removeAllCachedResponses()
        WKWebsiteDataStore.default().removeData(ofTypes: WKWebsiteDataStore.allWebsiteDataTypes(), modifiedSince: .distantPast) {}
        selection = nil
        state = nil
        apps = []
        widgets = [:]
        badges = [:]
        arrangements = [:]
        path = []
        current = "user"
        sheet = nil
        phase = .connecting
    }

    var config: Config? { state?.config }
    var install: Install? { state?.install }
    var me: Person? { state?.user }

    func app(_ name: String) -> AppInfo? { apps.first { $0.name == name } }
    /// What a person calls an app that uses an item: its label, or Backups for the node's own (a place to back up to).
    func label(_ name: String) -> String { name == "node" ? String(localized: "Backups") : app(name)?.label ?? name }

    // MARK: Pages

    /// Every page this user can read: theirs, their groups' in their order, then everyone's.
    var allPages: [Page] {
        guard let install, let me else { return [] }
        let order = arrangements["user"]?.pages?.order ?? []
        let mine = install.groups.filter { $0.role(of: me.id) != nil }
            .sorted { (order.firstIndex(of: "group:\($0.id)") ?? .max, $0.name) < (order.firstIndex(of: "group:\($1.id)") ?? .max, $1.name) }
        var pages = [Page(scope: "user", title: String(localized: "You"))]
        pages += mine.map { Page(scope: "group:\($0.id)", title: $0.name) }
        pages.append(Page(scope: "global", title: String(localized: "Everyone")))
        return pages
    }

    /// The pages shown: the groups' pages this user hid left out.
    var pages: [Page] {
        let hidden = arrangements["user"]?.pages?.hidden ?? []
        return allPages.filter { !hidden.contains($0.scope) }
    }

    func page(_ scope: String) -> Page? { allPages.first { $0.scope == scope } }

    /// The apps on a page: those installed in its scope that have a screen.
    func apps(on page: Page) -> [AppInfo] {
        apps.filter { $0.hasScreens && $0.scopes.contains(page.scope) }
    }

    /// A page's services, which its Services folder holds instead of each having an icon.
    func services(on page: Page) -> [AppInfo] {
        apps.filter { $0.isService && $0.scopes.contains(page.scope) }
            .sorted { $0.label.localizedStandardCompare($1.label) == .orderedAscending }
    }

    static func params(_ fields: [String: Any]) -> String {
        (try? JSONSerialization.data(withJSONObject: fields, options: .sortedKeys)).map { String(decoding: $0, as: UTF8.self) } ?? "{}"
    }

    /// The count an app's icon shows on a page, if any.
    func appBadge(_ app: String, on page: Page) -> Int? {
        badges[page.scope]?[app].flatMap { $0 > 0 ? $0 : nil }
    }

    /// Every widget a page could show: those of the apps installed in its scope, screen or not, at their defaults.
    func widgets(on page: Page) -> [WidgetInfo] {
        let here = Set(apps.filter { $0.scopes.contains(page.scope) }.map(\.name))
        return (widgets[page.scope] ?? []).filter { here.contains($0.app) }
    }

    /// How this user arranged each page, as their device last said.
    private(set) var arrangements: [String: Arrangement] = [:]

    /// The widgets a page's person put there, in their order, each at its size with its options. A page they never
    /// arranged has none.
    func arranged(on page: Page) -> [WidgetInfo] {
        let widgets = widgets(on: page)
        return (arrangements[page.scope]?.items ?? []).compactMap { item in
            guard let name = item.widget, item.id != nil, let widget = widgets.first(where: { $0.app == item.app && $0.widget == name }) else { return nil }
            return placed(widget, item)
        }
    }

    /// A widget as one copy of it on a page draws: at its size, with the options chosen for it.
    private func placed(_ widget: WidgetInfo, _ item: Arrangement.Item) -> WidgetInfo {
        var w = widget
        w.instance = item.id
        if let size = item.size, widget.supported.contains(size) { w.size = size }
        if let mine = item.id.flatMap({ widget.placed?[$0] }) {
            if let sizes = mine.sizes {
                w.sizes = (w.sizes ?? [:]).merging(sizes) { $1 }
                if let tree = sizes[w.size ?? ""] ?? sizes.values.first { w.root = tree }
            }
            if let error = mine.error { w.error = error }
        }
        return w
    }

    // MARK: Widgets

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

    /// The widgets on every page, each widget once: for the menu bar.
    var allWidgets: [WidgetInfo] {
        var seen = Set<String>()
        return pages.flatMap { arranged(on: $0) }.filter { seen.insert($0.kind).inserted }
    }

    /// Opens an app's first screen, telling it which page it came from.
    func open(_ app: AppInfo, from page: Page?) -> Route {
        .screen(ScreenRoute(app: app.name, scope: page?.scope))
    }

    // MARK: Settings

    /// What Settings shows this user, in order, as the node says.
    var settingsItems: [SettingsItem] { state?.settingsItems ?? [] }

    func settingsItem(_ name: String) -> SettingsItem? { settingsItems.first { $0.name == name } }

    /// What waits for this user: invites, devices to trust and items asked for.
    var requests: Int { settingsItem("requests")?.badge ?? 0 }

    /// The count on the app's own icon, as iOS adds up a folder's: every app's badge on every page, and the requests.
    var iconBadge: Int { requests + badges.values.flatMap(\.values).reduce(0) { $0 + max($1, 0) } }

    // MARK: Connection

    func run() async {
        drawKept()
        var misses = 0
        while !Task.isCancelled {
            let started = Date.now
            if await connect() {
                do {
                    for try await event in node.events() { await handle(event) }
                } catch {}
            }
            // At once after a stream that ran a while and ended (its node handing over to a new build), then waiting
            // longer each miss.
            if Date.now.timeIntervalSince(started) > 10 { misses = 0 }
            try? await Task.sleep(for: .seconds(Retry.wait(misses)))
            misses += 1
        }
    }

    /// This device isn't let in yet: the window is its approval screen, showing `code`, and not an app's screen or page,
    /// which would only say it can't reach anything. Once approved it opens on the person's own page.
    private func waitToBeApproved(code: String) {
        phase = .joining(code)
        sheet = nil
        path = []
        current = "user"
        #if os(macOS)
        selection = nil
        #else
        screenSheets = []
        #endif
    }

    /// Whether the node answered.
    private func connect() async -> Bool {
        let failure: Error
        do {
            try await node.transport.join { self.waitToBeApproved(code: $0) }
            try await refreshState()
            return true
        } catch is NotLetIn {
            // Not an unreachable node: the device was taken out of, or isn't yet in, the install, so it joins again.
            return false
        } catch is CancellationError {
            // Its person signed out meanwhile.
            return false
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

    /// The window as it was last (`KeptWindow`), drawn before the node first answers.
    private func drawKept() {
        guard state == nil, signedIn, let state = KeptWindow.read("state", as: NodeState.self), state.install != nil else { return }
        self.state = state
        apps = KeptWindow.read("apps", as: [AppInfo].self) ?? []
        widgets = KeptWindow.read("widgets", as: [String: [WidgetInfo]].self) ?? [:]
        arrangements = state.arrangements ?? [:]
        phase = .ready
        if !apps.isEmpty { openWhereItWas() }
    }

    /// The Mac's window opens where it was, unless a link already chose where.
    private func openWhereItWas() {
        #if os(macOS)
        if selection == nil {
            let kept = UserDefaults.standard.data(forKey: "selection").flatMap { try? JSONDecoder().decode(Route.self, from: $0) }
            selection = kept.flatMap(stillThere) ?? .page("user")
        }
        #endif
    }

    /// Whether the node has answered since the app opened (or its person signed in).
    private var answered = false

    func refreshState() async throws {
        // The first time, the apps are asked for beside the state, not after it.
        let node = node, began = session
        let early: Task<[AppInfo]?, Never>? = !answered && signedIn ? Task {
            guard let answer = try? await node.apps() else { return nil }
            return try? self.kept("apps", answer, since: began)
        } : nil
        let answer = try await node.state()
        let state = try kept("state", answer, since: began)
        let first = !answered
        answered = true
        let arrived = self.state?.install?.joining != false && state.install?.joining == false
        let regrouped = state.user != self.state?.user || state.install?.groups != self.state?.install?.groups
        self.state = state
        arrangements = state.arrangements ?? [:]
        phase = .ready
        if first, signedIn {
            let developer = (try? await DeviceClient.shared.developer(nil)) ?? developer
            try current(began)
            self.developer = developer
            await renameIfHostname()
            // Its pushes, now it's one of the install's devices, which alone the push Worker takes.
            Task { await Push.setUp() }
        }
        if first || arrived || apps.isEmpty || regrouped {
            let early = await early?.value
            try current(began)
            await refreshApps(early: early)
        }
    }

    private func handle(_ event: NodeEvent) async {
        if let json = try? JSONEncoder().encode(event) {
            NotificationCenter.default.post(name: .nodeEvent, object: String(decoding: json, as: UTF8.self))
        }
        if ["devices", "install"].contains(event.type) { try? await refreshState() }
        // Installs, deploys and data all arrive as changes; the node's own entries (a request for an item) change no app.
        if event.type == "changed" {
            let apps = event.apps ?? []
            if apps.isEmpty { try? await refreshState() }
            await refreshApps()
        }
    }

    /// Shows an app's screen in the window, over whatever was open: on the Mac in place of the page, on the iPhone
    /// pushed onto the page.
    func show(_ screen: ScreenRoute) {
        sheet = nil
        #if os(macOS)
        selection = .screen(screen)
        #else
        path = [.screen(screen)]
        #endif
    }

    /// `<cli>://vault?id=…&scope=…`, which an agent gives a person
    /// to answer a request for an item. Whether it was one.
    @discardableResult
    func open(_ url: URL) -> Bool {
        let query = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems ?? []
        let value = { (name: String) in query.first { $0.name == name }?.value ?? "" }
        guard url.host() == "vault", !value("id").isEmpty, !value("scope").isEmpty else { return false }
        sheet = .vault(scope: value("scope"), id: value("id"))
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

    func refreshApps(early: [AppInfo]? = nil) async {
        guard install != nil, signedIn else { return }
        let began = session
        do {
            let apps = if let early { early } else { try kept("apps", try await node.apps(), since: began) }
            if apps != self.apps { self.apps = apps }
            // A node that just started lists its deployed apps before it can describe them.
            if apps.contains(where: { ($0.deploys ?? 0) > 0 && $0.title == nil }) { throw NodeError("starting") }
        } catch is CancellationError {
            return
        } catch {
            Task {
                try? await Task.sleep(for: .seconds(2))
                if began == session { await refreshApps() }
            }
        }
        openWhereItWas()
        await refreshWidgets()
    }

    func refreshWidgets() async {
        guard let install, !install.joining, signedIn else { return }
        let node = node, began = session
        var fresh: [String: [WidgetInfo]] = [:]
        var answered = true
        await withTaskGroup(of: (String, [WidgetInfo]?).self) { tasks in
            for page in allPages { tasks.addTask { (page.scope, try? await node.widgets(scope: page.scope)) } }
            for await (scope, widgets) in tasks {
                fresh[scope] = widgets ?? self.widgets[scope] ?? []
                if widgets == nil { answered = false }
            }
        }
        guard began == session else { return }
        if fresh != widgets {
            widgets = fresh
            if let data = try? JSONEncoder().encode(fresh) { KeptWindow.keep("widgets", data) }
        }
        if answered { widgetsFetched = .now }
        scheduleRedraw()
        var counts: [String: [String: Int]] = [:]
        await withTaskGroup(of: (String, [String: Int]).self) { tasks in
            for page in allPages { tasks.addTask { (page.scope, (try? await node.badges(scope: page.scope)) ?? [:]) } }
            for await (scope, count) in tasks { counts[scope] = count }
        }
        guard began == session else { return }
        if counts != badges { badges = counts }
    }

    /// A route kept from before, if it still leads somewhere.
    private func stillThere(_ route: Route) -> Route? {
        switch route {
        case .page(let scope): pages.contains { $0.scope == scope } ? route : nil
        // A screen the app no longer has (Gmail's main, once it was a service) isn't; one not described yet may be.
        case .screen(let screen): app(screen.app).map { $0.screens == nil || $0.has(screen: screen.screen) } == true ? route : nil
        case .services(let scope): page(scope) != nil ? route : nil
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
