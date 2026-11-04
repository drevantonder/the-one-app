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
    /// link); `<cli>://page?scope=…` a page, or its Services with `folder=services`; `<cli>://settings`, or
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
            store.selection = value("folder") == "services" ? .services(scope) : .page(scope)
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

/// The Mac's window before and after its person signs in. Signed out, it's the sign-in screen and nothing from before.
/// Signed in, its calls carry their token to this Mac's own node, which opens their data with keys the vault gives
/// that sign-in, as for every other client; the node itself, and the sidecars, stay this machine's own programs.
@MainActor @Observable
final class Mac {
    static let shared = Mac()

    let store = Store(node: NodeClient(transport: MacNode(), client: "mac"))
    private(set) var signingIn = false
    var signInError: String?
    private var loopback: Loopback?
    private var running: Task<Void, Never>?

    init() {
        store.signedIn = Auth.signedIn
        store.startNode = NodeLauncher.start
        store.preferredDeviceName = Host.current().localizedName
    }

    /// Keeps the window in step with the node, as whoever is signed in.
    func run() {
        running?.cancel()
        running = Task { await store.run() }
    }

    /// Signs in at `typed` (an address, or an invite link, whose page makes the person's passkey first), with their
    /// passkey in the person's own browser, the default one, where their passkey is kept; the window comes to the front
    /// when they're back.
    func signIn(_ typed: String) async -> URL? {
        guard !signingIn else { return nil }
        signingIn = true
        signInError = nil
        defer { signingIn = false }
        do {
            if let invite = try Auth.take(typed) { return invite }
            let name = displayName(store.device?.name ?? Host.current().localizedName ?? "Mac")
            let loopback = try Loopback()
            self.loopback = loopback
            defer { self.loopback = nil }
            let signedIn = try await Auth.shared.signIn(name: "\(appName) on \(name)", redirect: loopback.redirect) { url in
                guard NSWorkspace.shared.open(url) else { throw NodeError(String(localized: "Couldn’t open your browser.")) }
                guard let back = await loopback.callback(appName: appName) else { throw Auth.Cancelled() }
                Self.comeToFront()
                return back
            }
            guard signedIn else { return nil }
        } catch {
            signInError = error.localizedDescription
            return nil
        }
        store.signedIn = true
        store.forget()
        run()
        return nil
    }

    /// Gives up waiting for the browser.
    func cancelSignIn() { loopback?.stop() }

    /// The window in front of the browser the person signed in with, which an app in the background can't put there by
    /// activating itself, but `open` can.
    private static func comeToFront() {
        let open = Process()
        open.executableURL = URL(filePath: "/usr/bin/open")
        open.arguments = ["-a", Bundle.main.bundlePath]
        try? open.run()
    }

    /// Signs out: the first screen again, with nothing of the person's left in the window.
    func signOut() async {
        await DeviceClient.shared.signOut()
        await Auth.shared.signOut()
        signedOut(nil)
    }

    /// The node no longer takes this Mac's sign-in.
    func signedOut(_ why: String?) {
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
        if Self.signedOut.contains(path) { return HTTPNode(base: Self.base) }
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

    func kept(app: String, op: String, input: Data) async -> (answer: Data, at: Int64)? {
        guard let node = try? node("") else { return nil }
        return await node.kept(app: app, op: op, input: input)
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
