import ServiceManagement
import SwiftUI

@main
struct MacApp: App {
    @State private var store: Store

    init() {
        SingleCopy.claim()
        let store = Store(node: .local)
        store.startNode = NodeLauncher.start
        store.preferredDeviceName = Host.current().localizedName
        Task { await store.run() }
        Sidecars.start()
        WidgetItems.shared.start(store)
        _store = State(initialValue: store)
        if !UserDefaults.standard.bool(forKey: "loginItemSet") {
            UserDefaults.standard.set(true, forKey: "loginItemSet")
            try? SMAppService.mainApp.register()
        }
    }

    /// `<cli>://open?app=…&screen=…&params=…&scope=…` opens an app's screen in the window, as on a phone (a widget's
    /// link); `<cli>://page?scope=…` a page, its Services folder over it with `folder=services`; `<cli>://settings`, or
    /// `…/settings/<item>`, opens Settings or one of its views.
    private func open(_ url: URL) {
        let query = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems ?? []
        let value = { (name: String) in query.first { $0.name == name }?.value?.trimmingCharacters(in: .whitespaces) }
        // What a link opens shows in the window, not under a sheet left open.
        if ["open", "page", "settings"].contains(url.host() ?? "") { store.sheet = nil }
        if url.host() == "open", let app = value("app") {
            store.path = []
            store.selection = .screen(ScreenRoute(app: app, screen: value("screen") ?? "main", params: value("params") ?? "{}", scope: value("scope")))
        } else if url.host() == "page", let scope = value("scope") {
            store.path = []
            store.selection = .page(scope)
            if value("folder") == "services" { store.sheet = .services(scope) }
        } else if url.host() == "settings" {
            let path: [Route] = url.pathComponents.dropFirst().first.flatMap(Setting.named).map { [.setting($0)] } ?? []
            if store.selection == .settings {
                store.path = path
            } else {
                store.selection = .settings
                // Settings' stack is new: it takes a path once it's shown.
                Task { @MainActor in store.path = path }
            }
        }
    }

    var body: some Scene {
        Window(appName, id: "main") {
            RootView()
                .environment(store)
                .tint(accent)
                .frame(minWidth: 760, minHeight: 500)
                .onOpenURL { if !store.open($0) { open($0) } }
                // Every app's badge and the requests on the Dock icon, as Mail counts unread mail.
                .onChange(of: store.iconBadge, initial: true) { NSApp.dockTile.badgeLabel = store.iconBadge > 0 ? "\(store.iconBadge)" : nil }
        }
        .defaultSize(width: 1000, height: 700)
        .commands {
            CommandGroup(before: .sidebar) {
                Button(store.sidebarFolded ? "Unfold Sidebar" : "Fold Sidebar to Icons") { withAnimation(.snappy) { store.sidebarFolded.toggle() } }
                    .keyboardShortcut("\\", modifiers: .command)
            }
            CommandGroup(replacing: .newItem) {
                Button("Pair a Phone or Computer…") { store.sheet = .pair }
                    .keyboardShortcut("i", modifiers: [.command, .shift])
                    .disabled(store.install == nil)
                Button("Join with a Pairing Code…") { store.sheet = .join }
                    .keyboardShortcut("j", modifiers: [.command, .shift])
                    .disabled(store.install != nil)
            }
        }

        MenuBarExtra {
            MenuPanel()
                .environment(store)
                .tint(accent)
        } label: {
            MenuBarMark(store: store)
        }
        .menuBarExtraStyle(.window)
    }

    private var accent: Color {
        store.config.map { Color(accentHex: $0.accent) } ?? .accentColor
    }
}

/// Keeps this device's node running: a login-persistent launch agent, or a child process if that's refused.
@MainActor
enum NodeLauncher {
    static var cli: String { Bundle.main.object(forInfoDictionaryKey: "CLI") as? String ?? "" }
    static var label: String { "\(Bundle.main.bundleIdentifier ?? "").node" }
    private static var child: Process?

    static func start() async throws {
        let agent = SMAppService.agent(plistName: "\(label).plist")
        if agent.status == .enabled {
            let kick = Process()
            kick.executableURL = URL(filePath: "/bin/launchctl")
            kick.arguments = ["kickstart", "gui/\(getuid())/\(label)"]
            try? kick.run()
            return
        }
        do {
            try agent.register()
            if agent.status == .enabled { return }
        } catch {}
        guard child?.isRunning != true else { return }
        let process = Process()
        process.executableURL = Bundle.main.bundleURL.appending(path: "Contents/MacOS/\(cli)")
        process.arguments = ["serve"]
        try process.run()
        child = process
    }
}

/// Only one copy of the app runs. A copy that starts while others run asks them to quit (after an update, the new
/// one is the one to keep) and stops any that won't; but a copy outside ~/Applications, such as one an update left
/// half staged, defers to the one there and quits itself.
enum SingleCopy {
    static func claim() {
        let me = NSRunningApplication.current
        let others = NSRunningApplication.runningApplications(withBundleIdentifier: Bundle.main.bundleIdentifier ?? "").filter { $0 != me }
        guard !others.isEmpty else { return }
        let home = FileManager.default.homeDirectoryForCurrentUser.appending(path: "Applications").standardizedFileURL.path
        let atHome = { (url: URL?) in url?.deletingLastPathComponent().standardizedFileURL.path == home }
        if !atHome(Bundle.main.bundleURL), let keeper = others.first(where: { atHome($0.bundleURL) }) {
            keeper.activate()
            exit(0)
        }
        others.forEach { $0.terminate() }
        let deadline = Date.now.addingTimeInterval(5)
        while others.contains(where: { !$0.isTerminated }), Date.now < deadline {
            RunLoop.current.run(until: .now.addingTimeInterval(0.1))
        }
        others.filter { !$0.isTerminated }.forEach { $0.forceTerminate() }
    }
}
