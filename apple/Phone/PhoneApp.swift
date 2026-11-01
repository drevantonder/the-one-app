import SwiftUI

@main
struct PhoneApp: App {
    @UIApplicationDelegateAdaptor(PhoneDelegate.self) private var delegate
    @State private var phone = Phone.shared

    init() {
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
                .tint(phone.accent)
                .task {
                    delegate.phone = phone
                    await phone.start()
                }
                .onOpenURL { phone.open($0) }
                .onChange(of: phone.store.config?.push, initial: true) { Task { await phone.setUpPush() } }
                .onChange(of: phone.store.iconBadge) { phone.showBadge(phone.store.iconBadge) }
        }
    }
}

/// The phone before and after it joins an install.
@MainActor @Observable
final class Phone {
    static let shared = Phone()

    enum Stage: Equatable {
        case starting
        case failed(String)
        case welcome
        case joined
    }

    private(set) var stage = Stage.starting
    private(set) var joining = false
    var joinError: String?
    let store = Store(node: Phone.transport)
    private var running: Task<Void, Never>?

    /// The accent: the build's until a server sends its own.
    var accent: Color {
        Color(accentHex: store.config?.accent ?? Bundle.main.object(forInfoDictionaryKey: "Accent") as? String ?? "")
    }

    /// In the simulator, NODE_URL points the app at a server's local API instead of joining over the network.
    static var nodeURL: URL? {
        #if targetEnvironment(simulator)
        ProcessInfo.processInfo.environment["NODE_URL"].flatMap(URL.init(string:))
        #else
        nil
        #endif
    }

    private static var transport: NodeClient {
        NodeClient(transport: nodeURL.map { HTTPNode(base: $0) as any NodeTransport } ?? ClientTransport())
    }

    private var starting: Task<Void, Never>?

    /// Starts the client, once, however many ask.
    func start() async {
        if starting == nil { starting = Task { await begin() } }
        await starting?.value
    }

    private func begin() async {
        if Self.nodeURL != nil {
            stage = .joined
            run()
            // OPEN_URL opens a link once it's up, as tapping it would: for screenshots.
            #if targetEnvironment(simulator)
            if let link = ProcessInfo.processInfo.environment["OPEN_URL"].flatMap(URL.init(string:)) {
                try? await Task.sleep(for: .seconds(3))
                open(link)
            }
            #endif
            return
        }
        let dir = URL.applicationSupportDirectory.appending(path: "client")
        do {
            stage = try await Client.start(in: dir) ? .joined : .welcome
        } catch {
            stage = .failed(error.localizedDescription)
        }
        if stage == .joined { run() }
    }

    /// Joins the install a pairing code is for, as `name`, and shows it.
    func join(_ code: String, name: String) async {
        guard !joining else { return }
        joining = true
        joinError = nil
        defer { joining = false }
        do {
            try await Client.join(code.trimmingCharacters(in: .whitespacesAndNewlines), name: name)
        } catch {
            joinError = joinFailure(error)
            return
        }
        stage = .joined
        run()
        try? await store.refreshState()
        // A device already in the install keeps the name it had there.
        if let current = store.state?.machine.name, current != name { try? await store.rename(name) }
    }

    /// `<cli>://join?code=…&name=…`: joins without the camera, from a link another device gives. Or a link the
    /// store opens itself, like an agent's request for a Vault item.
    /// `<cli>://open?app=…&screen=…&params=…&scope=…`: opens an app's screen, from a widget, on the page it came from.
    func open(_ url: URL) {
        if store.open(url) { return }
        let query = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems ?? []
        let value = { (name: String) in query.first { $0.name == name }?.value?.trimmingCharacters(in: .whitespaces) }
        // `<cli>://page?scope=…` turns to a page, `<cli>://settings` (or `…/settings/<item>`) to Settings.
        if url.host() == "page", let scope = value("scope") {
            store.path = []
            store.current = scope
            return
        }
        if url.host() == "settings" {
            store.current = "settings"
            store.path = url.pathComponents.dropFirst().first.flatMap(Setting.named).map { [.setting($0)] } ?? []
            return
        }
        if url.host() == "open", let app = value("app") {
            let scope = value("scope"), screen = value("screen") ?? "main"
            if screen == "main", store.app(app)?.hasScreens == false { return }
            if let scope, store.pages.contains(where: { $0.scope == scope }) { store.current = scope }
            store.path = [.screen(ScreenRoute(app: app, screen: screen, params: value("params") ?? "{}", scope: scope))]
            return
        }
        guard url.host() == "join", let code = value("code"), !code.isEmpty else { return }
        let name = value("name").flatMap { $0.isEmpty ? nil : $0 } ?? store.state?.machine.name ?? UIDevice.current.name
        Task { await join(code, name: name) }
    }

    private func run() {
        guard running == nil else { return }
        running = Task { await store.run() }
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
        case .starting:
            ProgressView()
        case .failed(let message):
            ContentUnavailableView("Can’t Start", systemImage: "exclamationmark.triangle", description: Text(message))
        case .welcome:
            WelcomeView()
        case .joined:
            PagesView()
        }
    }
}
