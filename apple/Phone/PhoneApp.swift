import SwiftUI
import AuthenticationServices
import WidgetKit

@main
struct PhoneApp: App {
    @UIApplicationDelegateAdaptor(PhoneDelegate.self) private var delegate
    @State private var phone = Phone.shared

    init() {
        Icons.load()
        // A home-screen widget's button or toggle starts the app in the background, if it isn't running, to run it.
        WidgetActions.run = { app, op, input, _ in
            let phone = Phone.shared
            await phone.start()
            _ = try await phone.store.node.call(app: app, op: op, input: input)
            await phone.store.refreshWidgets()
            await phone.store.saveForHomeScreen()
        }
    }

    var body: some Scene {
        WindowGroup {
            PhoneRoot()
                .environment(phone)
                .environment(phone.store)
                .task {
                    delegate.phone = phone
                    await phone.start()
                }
                .task {
                    // Once the first frame is up.
                    try? await Task.sleep(for: .milliseconds(300))
                    WarmWebKit.start()
                }
                .onOpenURL { phone.open($0) }
                .onChange(of: phone.store.iconBadge) { phone.showBadge(phone.store.iconBadge) }
        }
    }
}

/// The phone before and after it signs in.
@MainActor @Observable
final class Phone {
    static let shared = Phone()

    enum Stage: Equatable {
        case welcome
        case joined
    }

    private(set) var stage = Phone.nodeURL != nil || Auth.signedIn ? Stage.joined : .welcome
    private(set) var signingIn = false
    var joinError: String?
    let store = Store(node: NodeClient(transport: PhoneNode(), client: "phone"))
    private var running: Task<Void, Never>?

    /// In the simulator, NODE_URL points the app at a device's local API instead of signing in.
    nonisolated static var nodeURL: URL? {
        #if targetEnvironment(simulator)
        ProcessInfo.processInfo.environment["NODE_URL"].flatMap(URL.init(string:))
        #else
        nil
        #endif
    }

    private var starting: Task<Void, Never>?

    /// Starts, once, however many ask.
    func start() async {
        if starting == nil { starting = Task { await begin() } }
        await starting?.value
    }

    private func begin() async {
        guard stage == .joined else { return }
        run()
        // OPEN_URL opens a link once it's up, as tapping it would: for screenshots.
        #if targetEnvironment(simulator)
        if let link = ProcessInfo.processInfo.environment["OPEN_URL"].flatMap(URL.init(string:)) {
            try? await Task.sleep(for: .seconds(3))
            open(link)
        }
        #endif
    }

    /// Signs in to the install at `typed` (an address, or an invite link, whose page makes the person's passkey
    /// first), with their passkey in the system's sign-in sheet.
    func signIn(_ typed: String, session: WebAuthenticationSession) async -> URL? {
        guard !signingIn else { return nil }
        signingIn = true
        joinError = nil
        defer { signingIn = false }
        do {
            if let invite = try Auth.take(typed) { return invite }
            let signedIn = try await Auth.shared.signIn(name: "\(appName) on \(UIDevice.current.name)") { url in
                try await session.authenticate(using: url, callback: .customScheme(Auth.cli), preferredBrowserSession: .shared, additionalHeaderFields: [:])
            }
            guard signedIn else { return nil }
        } catch {
            joinError = error.localizedDescription
            return nil
        }
        stage = .joined
        run()
        return nil
    }

    /// Signs out: back to the first screen, with nothing of the person's left on the phone, and no more of their pushes
    /// (the push Worker forgets a device the install no longer has).
    func signOut() async {
        await DeviceClient.shared.signOut()
        await Auth.shared.signOut()
        signedOut(nil)
    }

    /// The install no longer takes this phone's sign-in. A call that fails once the person signed out says nothing.
    func signedOut(_ why: String?) {
        if stage == .welcome, why != nil { return }
        running?.cancel()
        running = nil
        starting = nil
        stage = .welcome
        joinError = why
        store.forget()
        WidgetCache.forget()
        WidgetCenter.shared.reloadAllTimelines()
    }

    /// `<cli>://open?app=…&screen=…&params=…&scope=…`: opens an app's screen, from a widget, on the page it came
    /// from. Or a link the store opens itself, like an agent's request for a Vault item.
    func open(_ url: URL) {
        if store.open(url) { return }
        let query = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems ?? []
        let value = { (name: String) in query.first { $0.name == name }?.value?.trimmingCharacters(in: .whitespaces) }
        // `<cli>://page?scope=…` turns to a page (opening its Accounts with `folder=accounts`), `<cli>://settings` (or
        // `…/settings/<item>`) to Settings.
        if url.host() == "page", let scope = value("scope") {
            store.path = value("folder") == "accounts" ? [.accounts(scope)] : []
            store.current = scope
            return
        }
        if url.host() == "settings" {
            store.current = "settings"
            store.path = url.pathComponents.dropFirst().first.map { [.setting($0)] } ?? []
            return
        }
        if url.host() == "open", let app = value("app") {
            let scope = value("scope"), screen = value("screen") ?? "main"
            if screen == "main", store.app(app)?.hasScreens == false { return }
            // Its page, or once the pages come if they haven't yet (one that isn't there then turns to the person's own).
            if let scope, store.pages.isEmpty || store.pages.contains(where: { $0.scope == scope }) { store.current = scope }
            store.path = [.screen(ScreenRoute(app: app, screen: screen, params: value("params") ?? "{}", scope: scope))]
        }
    }

    private func run() {
        guard running == nil else { return }
        running = Task { await store.run() }
    }
}

/// The person's nodes, over iroh as this phone's own device: or in the simulator, NODE_URL's node as its own.
struct PhoneNode: NodeTransport {
    private var node: any NodeTransport {
        if let url = Phone.nodeURL { return HTTPNode(base: url) }
        return DeviceNode()
    }

    var unreachable: String { node.unreachable }

    private func signedOut<T>(_ work: () async throws -> T) async throws -> T {
        do {
            return try await work()
        } catch let error as Auth.SignedOut {
            await Phone.shared.signedOut(error.message)
            throw error
        }
    }

    func send(_ method: String, _ path: String, body: Data?, timeout: TimeInterval) async throws -> Data {
        try await signedOut { try await node.send(method, path, body: body, timeout: timeout) }
    }

    func page(app: String, screen: String, params: String, scope: String?) async throws -> String {
        try await signedOut { try await node.page(app: app, screen: screen, params: params, scope: scope) }
    }

    func events() -> AsyncThrowingStream<NodeEvent, Error> {
        node.events()
    }

    func kept(app: String, op: String, input: Data) async -> (answer: Data, at: Int64)? {
        await node.kept(app: app, op: op, input: input)
    }

    func join(showing: @MainActor @Sendable (String) -> Void) async throws {
        guard Phone.nodeURL == nil else { return }
        let name = await UIDevice.current.name
        try await signedOut { try await DeviceNode().join(name: name, showing: showing) }
    }
}

struct PhoneRoot: View {
    @Environment(Phone.self) private var phone
    @AppStorage("appearance") private var appearance = Appearance.automatic.rawValue

    var body: some View {
        stage.preferredColorScheme(Appearance(rawValue: appearance)?.scheme)
    }

    @ViewBuilder private var stage: some View {
        switch phone.stage {
        case .welcome:
            WelcomeView()
        case .joined:
            ScreensHome()
        }
    }
}
