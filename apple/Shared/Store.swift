import Foundation
import Observation
import WebKit

/// A page: one scope the user can read, showing the apps installed there and their widgets.
struct Page: Identifiable, Hashable {
    /// `user`, `group:<id>` or `global`: also the hint a screen opened from here gets as `one.scope`.
    var scope: String
    var title: String
    var subtitle: String

    var id: String { scope }
    var group: String? { scope.hasPrefix("group:") ? String(scope.dropFirst(6)) : nil }
    /// Whose it is, as a folder's subtitle starts: "For you only".
    var whose: String {
        switch scope {
        case "user": String(localized: "For you only")
        case "global": String(localized: "For everyone")
        default: String(localized: "For \(title)")
        }
    }
}

/// One thing on a page's grid: an app's icon, or a widget its person put there.
enum PageItem: Identifiable, Equatable {
    case app(AppInfo)
    case widget(WidgetInfo, Arrangement.Item)

    var id: String {
        switch self {
        case .app(let app): "app:\(app.name)"
        case .widget(_, let item): "widget:\(item.id ?? "")"
        }
    }

    /// How an arrangement keeps it.
    var kept: Arrangement.Item {
        switch self {
        case .app(let app): Arrangement.Item(app: app.name)
        case .widget(_, let item): item
        }
    }
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
    case pages
    /// A request for a Vault item, from a `<cli>://vault?id=…&scope=…` link, or an app screen's `one.vault(need)`.
    case vault(scope: String, id: String)
    /// The shell's own request for an item, made here rather than as an entry (push's Apple key, from Settings).
    case ask(VaultRequest)
    /// The widget gallery, adding to a page: its scope.
    case addWidget(String)
    /// A page's Services folder, on a Mac: its scope.
    case services(String)

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
    /// Each page's apps' badges, by scope then app.
    private(set) var badges: [String: [String: Int]] = [:]
    /// The Mac's sidebar selection: the page, screen or view at the bottom of `path`. It's kept, so the app opens
    /// where it was, after an update too.
    var selection: Route? {
        didSet {
            guard selection != oldValue else { return }
            path = []
            editing = false
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
    #if os(iOS)
    /// Screens slid up over the others, by the bridge's `open` with `sheet`, each over the one before.
    var screenSheets: [ScreenSheet] = []
    #endif

    /// The order this user put their group pages in, and the ones they hid, on this machine.
    private(set) var pageOrder = UserDefaults.standard.stringArray(forKey: "pageOrder") ?? []
    private(set) var hiddenPages = Set(UserDefaults.standard.stringArray(forKey: "hiddenPages") ?? [])

    init(node: NodeClient) {
        self.node = node
    }

    /// Forgets everything it showed, and what it kept of the person's on this device: they signed out.
    func forget() {
        ScreenCache.forget()
        ScreenFile.forget()
        URLCache.shared.removeAllCachedResponses()
        WKWebsiteDataStore.default().removeData(ofTypes: WKWebsiteDataStore.allWebsiteDataTypes(), modifiedSince: .distantPast) {}
        for key in ["pageOrder", "hiddenPages"] { UserDefaults.standard.removeObject(forKey: key) }
        pageOrder = []
        hiddenPages = []
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
    var isAdmin: Bool { me?.admin == true }
    /// What to paste into an agent to connect it, as the node says.
    var askYourAgent: String { state?.agentPrompt ?? agentPrompt(address: nil) }

    func app(_ name: String) -> AppInfo? { apps.first { $0.name == name } }
    /// What a person calls an app that uses an item: its label, or Backups for the node's own (a place to back up to).
    func label(_ name: String) -> String { name == "node" ? String(localized: "Backups") : app(name)?.label ?? name }
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
            Page(scope: "group:\(group.id)", title: group.name, subtitle: "\(people(group.members.count)) · \(appsChosenBy(group.admins))")
        }
        pages.append(Page(scope: "global", title: String(localized: "Everyone"),
                          subtitle: "\(people(install.users.count)) · \(appsChosenBy(install.users.filter(\.admin).map(\.id)))"))
        return pages
    }

    /// The pages shown: hidden group pages left out.
    var pages: [Page] { allPages.filter { !hiddenPages.contains($0.scope) } }

    func page(_ scope: String) -> Page? { allPages.first { $0.scope == scope } }

    private func people(_ n: Int) -> String { n == 1 ? String(localized: "1 person") : String(localized: "\(n) people") }

    private func appsChosenBy(_ admins: [String]) -> String {
        let names = admins.sorted { ($0 == me?.id ? 1 : 0, userName($0)) < ($1 == me?.id ? 1 : 0, userName($1)) }
            .map { $0 == me?.id ? String(localized: "you") : userName($0) }
        switch names.count {
        case 0: return String(localized: "apps chosen by admins")
        case 1: return String(localized: "apps chosen by \(names[0])")
        case 2: return String(localized: "apps chosen by \(names[0]) and \(names[1])")
        default: return String(localized: "apps chosen by \(names.count) admins")
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

    /// A page's services, which its Services folder holds instead of each having an icon.
    func services(on page: Page) -> [AppInfo] {
        apps.filter { $0.isService && $0.scopes.contains(page.scope) }
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

    /// The App Store's page for an app, if the install's store app is here and the app is one it lists (sidecars aren't).
    func storeRoute(for app: AppInfo) -> Route? {
        guard let name = config?.store, let store = self.app(name), store.has(screen: "app"), (app.deploys ?? 0) > 0 else { return nil }
        return .screen(ScreenRoute(app: name, screen: "app", params: Self.params(["app": app.name])))
    }

    /// The App Store's Services tab, if the install's store app is here.
    var serviceStoreRoute: Route? {
        guard let name = config?.store, app(name)?.hasScreens == true else { return nil }
        return .screen(ScreenRoute(app: name, params: Self.params(["service": true])))
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

    /// Every widget a page could show: those of the apps installed in its scope, screen or not, at their defaults.
    /// The widget gallery offers them; a page shows those its person put there (`items(on:)`).
    func widgets(on page: Page) -> [WidgetInfo] {
        let here = Set(apps.filter { $0.scopes.contains(page.scope) }.map(\.name))
        return (widgets[page.scope] ?? []).filter { here.contains($0.app) }
    }

    // MARK: Arranging

    /// How this user arranged each page, as their server last said, with what changed here since that it hasn't yet.
    private(set) var arrangements: [String: Arrangement] = [:]
    private var unsaved: [String: Arrangement] = [:]
    private var saving: Task<Void, Never>?
    /// Whether the pages are being arranged: everything jiggles, on every page, as on the Home Screen.
    var editing = false
    /// Why the last change to a page didn't save, until someone sees it.
    var arrangeError: String?

    /// What a page shows, in its person's order: the widgets they put there and its apps' icons, those installed
    /// since they last arranged it at the end, without those they took off it. A page they never arranged shows its
    /// apps and no widgets.
    func items(on page: Page) -> [PageItem] {
        let apps = apps(on: page), widgets = widgets(on: page)
        guard let arrangement = arrangements[page.scope] else { return apps.map(PageItem.app) }
        var out: [PageItem] = [], seen = Set<String>()
        for item in arrangement.items {
            if let name = item.widget {
                guard let id = item.id, let widget = widgets.first(where: { $0.app == item.app && $0.widget == name }), seen.insert("widget:\(id)").inserted else { continue }
                out.append(.widget(placed(widget, item), item))
            } else if let app = apps.first(where: { $0.name == item.app }), seen.insert("app:\(app.name)").inserted {
                out.append(.app(app))
            }
        }
        let hidden = Set(arrangement.hidden ?? [])
        return out + apps.filter { !seen.contains("app:\($0.name)") && !hidden.contains($0.name) }.map(PageItem.app)
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

    /// The apps this user took off a page, which stay installed there.
    func hiddenApps(on page: Page) -> [AppInfo] {
        (arrangements[page.scope]?.hidden ?? []).compactMap(app).filter { $0.hasScreens && $0.scopes.contains(page.scope) }
    }

    /// Keeps a page in this order.
    func arrange(_ items: [PageItem], on page: Page) {
        save(Arrangement(items: items.map(\.kept), hidden: arrangements[page.scope]?.hidden ?? []), on: page.scope)
    }

    /// Takes a widget off a page, or an app's icon: the app stays installed there.
    func remove(_ item: PageItem, from page: Page) {
        var hidden = arrangements[page.scope]?.hidden ?? []
        if case .app(let app) = item, !hidden.contains(app.name) { hidden.append(app.name) }
        save(Arrangement(items: items(on: page).filter { $0 != item }.map(\.kept), hidden: hidden), on: page.scope)
    }

    /// Puts an app taken off a page back, at its end.
    func show(_ app: AppInfo, on page: Page) {
        let hidden = (arrangements[page.scope]?.hidden ?? []).filter { $0 != app.name }
        save(Arrangement(items: items(on: page).map(\.kept) + [Arrangement.Item(app: app.name)], hidden: hidden), on: page.scope)
    }

    /// Adds a copy of a widget to a page, at `size`, in the first place it fits.
    func add(_ widget: WidgetInfo, size: String, to page: Page) {
        let item = Arrangement.Item(app: widget.app, widget: widget.widget, id: Self.newID(), size: size)
        save(Arrangement(items: items(on: page).map(\.kept) + [item], hidden: arrangements[page.scope]?.hidden ?? []), on: page.scope)
    }

    func resize(_ item: Arrangement.Item, to size: String, on page: Page) {
        change(item, on: page) { $0.size = size }
    }

    /// Sets one of a widget's options on a page, or with nil puts back its default; the widget draws again with it.
    func setOption(_ name: String, _ value: JSONValue?, of item: Arrangement.Item, on page: Page) {
        change(item, on: page) { item in
            var options = item.options ?? [:]
            options[name] = value
            item.options = options.isEmpty ? nil : options
        }
    }

    private func change(_ item: Arrangement.Item, on page: Page, _ edit: (inout Arrangement.Item) -> Void) {
        let items = items(on: page).map(\.kept).map { kept in
            guard kept.id != nil, kept.id == item.id else { return kept }
            var changed = kept
            edit(&changed)
            return changed
        }
        save(Arrangement(items: items, hidden: arrangements[page.scope]?.hidden ?? []), on: page.scope, redraw: items.contains { $0.id == item.id && $0.options != nil })
    }

    /// Keeps an arrangement here at once, then saves it, after any saving before it, so the latest is the one kept.
    private func save(_ arrangement: Arrangement, on scope: String, redraw: Bool = false) {
        arrangements[scope] = arrangement
        unsaved[scope] = arrangement
        let before = saving
        saving = Task {
            await before?.value
            do {
                try await node.arrange(page: scope, arrangement)
            } catch {
                arrangeError = error.localizedDescription
            }
            if unsaved[scope] == arrangement { unsaved[scope] = nil }
            if redraw { await refreshWidgets() }
        }
    }

    static func newID() -> String { String(UUID().uuidString.replacingOccurrences(of: "-", with: "").prefix(10)).lowercased() }

    /// Whether this device showed pages before each person arranged their own, when every page showed all its apps'
    /// widgets but those hidden on the device: then the first time it finds a page not arranged yet, it keeps the
    /// widgets it showed there, at the sizes it showed them, as if chosen.
    private var showedPagesBefore = !UserDefaults.standard.bool(forKey: "pagesArranged")
        && ["selection", "widgetSizes", "pageHiddenWidgets"].contains { UserDefaults.standard.object(forKey: $0) != nil }
    /// The pages whose widgets came from their server, the last time they were asked for.
    private var loadedWidgets = Set<String>()

    private func keepWidgetsShownBefore() {
        guard showedPagesBefore, state?.arrangements != nil else { return }
        let defaults = UserDefaults.standard
        let hidden = Set(defaults.stringArray(forKey: "pageHiddenWidgets") ?? [])
        let sizes = defaults.dictionary(forKey: "widgetSizes") as? [String: String] ?? [:]
        var done = Set(defaults.stringArray(forKey: "pagesKept") ?? [])
        for page in allPages where !done.contains(page.scope) && loadedWidgets.contains(page.scope) {
            done.insert(page.scope)
            let shown = widgets(on: page).filter { !hidden.contains("\(page.scope)/\($0.kind)") }
            guard arrangements[page.scope] == nil, !shown.isEmpty else { continue }
            let items = shown.map { w in
                Arrangement.Item(app: w.app, widget: w.widget, id: Self.newID(), size: sizes["\(page.scope)/\(w.kind)"].flatMap { w.supported.contains($0) ? $0 : nil } ?? w.size)
            } + apps(on: page).map { Arrangement.Item(app: $0.name) }
            save(Arrangement(items: items, hidden: []), on: page.scope)
        }
        defaults.set(Array(done), forKey: "pagesKept")
        if allPages.allSatisfy({ done.contains($0.scope) }) {
            showedPagesBefore = false
            defaults.set(true, forKey: "pagesArranged")
            for key in ["pageHiddenWidgets", "widgetSizes", "pagesKept"] { defaults.removeObject(forKey: key) }
        }
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
        return pages.flatMap { items(on: $0) }.compactMap { item -> WidgetInfo? in
            if case .widget(let widget, _) = item { widget } else { nil }
        }.filter { seen.insert($0.kind).inserted }
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

    /// What Settings shows this user, in order, as the node says.
    var settingsItems: [SettingsItem] { state?.settingsItems ?? [] }

    func settingsItem(_ name: String) -> SettingsItem? { settingsItems.first { $0.name == name } }

    /// What waits for this user: invites, servers to trust and items asked for.
    var requests: Int { settingsItem("requests")?.badge ?? 0 }

    /// The count on Settings' icon: every item's, the requests waiting and backups' problem.
    var badge: Int { settingsItems.reduce(0) { $0 + ($1.badge ?? 0) } }

    /// The count on the app's own icon, as iOS adds up a folder's: every app's badge on every page, and the requests.
    var iconBadge: Int { requests + badges.values.flatMap(\.values).reduce(0) { $0 + max($1, 0) } }

    /// Whether Settings has something to see: requests waiting, or backups' problem.
    var needsAttention: Bool { badge > 0 }

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
        arrangements = (state.arrangements ?? [:]).merging(unsaved) { $1 }
        phase = .ready
        if first, signedIn { await renameIfHostname() }
        if first || arrived || apps.isEmpty || regrouped { await refreshApps() }
    }

    private func handle(_ event: NodeEvent) async {
        if let json = try? JSONEncoder().encode(event) {
            NotificationCenter.default.post(name: .nodeEvent, object: String(decoding: json, as: UTF8.self))
        }
        if ["servers", "install"].contains(event.type) { try? await refreshState() }
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

    func refreshApps() async {
        guard install != nil, signedIn else { return }
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
        guard let install, !install.joining, signedIn else { return }
        let node = node
        var fresh: [String: [WidgetInfo]] = [:]
        await withTaskGroup(of: (String, [WidgetInfo]?).self) { tasks in
            for page in allPages { tasks.addTask { (page.scope, try? await node.widgets(scope: page.scope)) } }
            for await (scope, widgets) in tasks {
                fresh[scope] = widgets ?? self.widgets[scope] ?? []
                if widgets != nil { loadedWidgets.insert(scope) }
            }
        }
        if fresh != widgets { widgets = fresh }
        keepWidgetsShownBefore()
        scheduleRedraw()
        var counts: [String: [String: Int]] = [:]
        for page in allPages { counts[page.scope] = (try? await node.badges(scope: page.scope)) ?? [:] }
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
