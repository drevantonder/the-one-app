import AuthenticationServices
import ServiceManagement
import SwiftUI

@main
struct MacApp: App {
    @State private var mac = Mac.shared
    private var store: Store { mac.store }

    init() {
        Icons.load()
        // The window shows at once; the rest waits for an older copy to quit.
        SingleCopy.claim {
            Mac.shared.run()
            Sidecars.start()
            WidgetItems.shared.start(Mac.shared.store)
        }
        if !UserDefaults.standard.bool(forKey: "loginItemSet") {
            UserDefaults.standard.set(true, forKey: "loginItemSet")
            Task.detached(priority: .utility) { try? SMAppService.mainApp.register() }
        }
    }

    /// `<cli>://open?app=…&screen=…&params=…&scope=…` opens an app's screen in the window, as on a phone (a widget's
    /// link); `<cli>://page?scope=…` a page, its Services folder over it with `folder=services`; `<cli>://settings`, or
    /// `…/settings/<item>`, opens Settings or one of its screens.
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
            store.path = []
            store.selection = url.pathComponents.dropFirst().first.map { .setting($0) } ?? .settings
        }
    }

    var body: some Scene {
        Window(appName, id: "main") {
            RootView()
                .environment(mac)
                .environment(store)
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
                Button("Pair a Computer…") { store.sheet = .pair }
                    .keyboardShortcut("i", modifiers: [.command, .shift])
                    .disabled(store.install == nil || !store.signedIn)
                Button("Join with a Pairing Code…") { store.sheet = .join }
                    .keyboardShortcut("j", modifiers: [.command, .shift])
                    .disabled(store.install != nil)
            }
        }

        MenuBarExtra {
            MenuPanel()
                .environment(store)
        } label: {
            MenuBarMark(store: store)
        }
        .menuBarExtraStyle(.window)
    }
}

/// The Mac's window before and after its person signs in. Signed in, its calls carry their token to this Mac's own
/// node, which opens their data with keys the vault gives that sign-in, as for every other client; the node itself,
/// and the sidecars, stay this machine's own programs.
@MainActor @Observable
final class Mac {
    static let shared = Mac()

    let store = Store(node: NodeClient(transport: MacNode(), client: "mac"))
    private(set) var signingIn = false
    var signInError: String?
    /// Asking as this machine's own program, until its person signs in.
    private(set) var asMachine = false
    private var running: Task<Void, Never>?

    init() {
        let defaults = UserDefaults.standard
        // A window that showed pages before it could sign in keeps asking as this machine's own program until its
        // person signs in, rather than an update signing them out.
        // Read from the Keychain once.
        let signedIn = Auth.signedIn
        if defaults.object(forKey: "asMachine") == nil { defaults.set(!signedIn && defaults.object(forKey: "selection") != nil, forKey: "asMachine") }
        asMachine = defaults.bool(forKey: "asMachine") && !signedIn
        store.signedIn = signedIn || asMachine
        store.startNode = NodeLauncher.start
        store.preferredDeviceName = Host.current().localizedName
    }

    /// Whether the window still asks as this machine's own program, not signed in. Goes once nodes refuse calls with
    /// no token (ARCHITECTURE.md).
    nonisolated static var asMachine: Bool { UserDefaults.standard.bool(forKey: "asMachine") && !Auth.signedIn }

    /// Keeps the window in step with the node, as whoever is signed in.
    func run() {
        running?.cancel()
        running = Task { await store.run() }
    }

    /// Signs in at `typed` (an address, or an invite link, whose page makes the person's passkey first), with their
    /// passkey in the system's sign-in sheet.
    func signIn(_ typed: String, session: WebAuthenticationSession) async -> URL? {
        guard !signingIn else { return nil }
        signingIn = true
        signInError = nil
        defer { signingIn = false }
        do {
            if let invite = try Auth.take(typed) { return invite }
            let name = displayName(store.device?.name ?? Host.current().localizedName ?? "Mac")
            // In a session of its own: the browser keeps nothing, and asks no "wants to use … to sign in" before the passkey.
            let signedIn = try await Auth.shared.signIn(name: "\(appName) on \(name)") { url in
                try await session.authenticate(using: url, callback: .customScheme(Auth.cli), preferredBrowserSession: .ephemeral, additionalHeaderFields: [:])
            }
            guard signedIn else { return nil }
        } catch {
            signInError = error.localizedDescription
            return nil
        }
        UserDefaults.standard.set(false, forKey: "asMachine")
        asMachine = false
        store.signedIn = true
        store.forget()
        run()
        return nil
    }

    /// Signs out: the first screen again, with nothing of the person's left in the window.
    func signOut() async {
        await Auth.shared.signOut()
        await DeviceClient.shared.close()
        signedOut(nil)
    }

    /// The node no longer takes this Mac's sign-in.
    func signedOut(_ why: String?) {
        UserDefaults.standard.set(false, forKey: "asMachine")
        asMachine = false
        guard store.signedIn else { return }
        store.signedIn = false
        signInError = why
        store.forget()
        run()
    }
}

/// Signed in, the person's nodes over iroh, as this window's own device (which this Mac's own node approves at once, if
/// it's theirs); signed out, this Mac's own node, asked only what it is, and to join an install with a pairing code.
struct MacNode: NodeTransport {
    private static let base = URL(string: "http://127.0.0.1:\(Bundle.main.object(forInfoDictionaryKey: "NodePort") as! String)")!
    private static let signedOut: Set = ["v1/state", "v1/join"]

    var unreachable: String { Auth.signedIn ? DeviceNode().unreachable : HTTPNode(base: Self.base).unreachable }

    /// The node, as whoever's signed in: none, for what only someone signed in asks.
    private func node(_ path: String) throws -> any NodeTransport {
        if Auth.signedIn { return DeviceNode() }
        if Mac.asMachine || Self.signedOut.contains(path) { return HTTPNode(base: Self.base) }
        throw Auth.SignedOut(message: String(localized: "Sign in first."))
    }

    private func signedIn<T>(_ path: String, _ work: (any NodeTransport) async throws -> T) async throws -> T {
        let node = try node(path)
        do {
            return try await work(node)
        } catch let error as Auth.SignedOut {
            if Auth.signedIn { await Mac.shared.signedOut(error.message) }
            throw error
        }
    }

    func send(_ method: String, _ path: String, body: Data?, timeout: TimeInterval) async throws -> Data {
        try await signedIn(path) { try await $0.send(method, path, body: body, timeout: timeout) }
    }

    func page(app: String, screen: String, params: String, scope: String?) async throws -> String {
        try await signedIn("") { try await $0.page(app: app, screen: screen, params: params, scope: scope) }
    }

    func settingsPage(_ name: String, params: String) async throws -> String {
        try await signedIn("") { try await $0.settingsPage(name, params: params) }
    }

    /// Signed out, none: the window is asked again once someone signs in.
    func events() -> AsyncThrowingStream<NodeEvent, Error> {
        guard let node = try? node("") else { return AsyncThrowingStream { _ in } }
        return node.events()
    }

    func join(showing: @MainActor @Sendable (String) -> Void) async throws {
        guard Auth.signedIn else { return }
        let name = "\(appName) on \(Host.current().localizedName ?? "Mac")"
        try await signedIn("") { _ in try await DeviceNode().join(name: name, showing: showing, approve: Self.approveHere) }
    }

    /// This Mac's own node approves the window by its code, as one of this Mac's own programs, if it's a device of the
    /// same person: else the code waits for another of theirs.
    @Sendable private static func approveHere(_ code: String) async {
        var request = URLRequest(url: base.appending(path: "v1/devices/approve"), timeoutInterval: 30)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try? JSONSerialization.data(withJSONObject: ["code": code.replacingOccurrences(of: " ", with: ""), "confirm": true])
        LocalToken.add(to: &request)
        _ = try? await URLSession.shared.data(for: request)
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
            if await kickstarted() { return }
            // Enabled, yet launchd won't run it, as once the app's been rebuilt since it was registered: register it
            // again, which records this build.
            try? await agent.unregister()
        }
        do {
            try agent.register()
            if agent.status == .enabled, await kickstarted() { return }
        } catch {}
        guard child?.isRunning != true else { return }
        let process = Process()
        process.executableURL = Bundle.main.bundleURL.appending(path: "Contents/MacOS/\(cli)")
        process.arguments = ["serve"]
        try process.run()
        child = process
    }

    /// Asks launchd to run the node's agent now: → whether it's running within a few seconds.
    private static func kickstarted() async -> Bool {
        _ = launchctl(["kickstart", "gui/\(getuid())/\(label)"])
        for _ in 0..<10 {
            if launchctl(["print", "gui/\(getuid())/\(label)"]).contains("state = running") { return true }
            try? await Task.sleep(for: .milliseconds(300))
        }
        return false
    }

    private static func launchctl(_ arguments: [String]) -> String {
        let process = Process()
        let out = Pipe()
        process.executableURL = URL(filePath: "/bin/launchctl")
        process.arguments = arguments
        process.standardOutput = out
        process.standardError = Pipe()
        guard (try? process.run()) != nil else { return "" }
        let said = out.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        return String(decoding: said, as: UTF8.self)
    }
}

/// Only one copy of the app runs. A copy that starts while others run asks them to quit (after an update, the new
/// one is the one to keep) and stops any that won't; but a copy outside ~/Applications, such as one an update left
/// half staged, defers to the one there and quits itself.
@MainActor
enum SingleCopy {
    /// Runs `then` once no other copy does.
    static func claim(then: @escaping @MainActor () -> Void) {
        let me = NSRunningApplication.current
        let others = NSRunningApplication.runningApplications(withBundleIdentifier: Bundle.main.bundleIdentifier ?? "").filter { $0 != me }
        guard !others.isEmpty else { return then() }
        let home = FileManager.default.homeDirectoryForCurrentUser.appending(path: "Applications").standardizedFileURL.path
        let atHome = { (url: URL?) in url?.deletingLastPathComponent().standardizedFileURL.path == home }
        if !atHome(Bundle.main.bundleURL), let keeper = others.first(where: { atHome($0.bundleURL) }) {
            keeper.activate()
            exit(0)
        }
        others.forEach { $0.terminate() }
        Task {
            let deadline = Date.now.addingTimeInterval(5)
            while others.contains(where: { !$0.isTerminated }), Date.now < deadline {
                try? await Task.sleep(for: .milliseconds(100))
            }
            others.filter { !$0.isTerminated }.forEach { $0.forceTerminate() }
            then()
        }
    }
}
