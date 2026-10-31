import SwiftUI

@main
struct PhoneApp: App {
    @State private var phone = Phone()

    var body: some Scene {
        WindowGroup {
            PhoneRoot()
                .environment(phone)
                .environment(phone.store)
                .tint(phone.accent)
                .task { await phone.start() }
                .onOpenURL { phone.open($0) }
        }
    }
}

/// The phone before and after it joins a space.
@MainActor @Observable
final class Phone {
    enum Stage: Equatable {
        case starting
        case failed(String)
        case welcome
        case joined
    }

    private(set) var stage = Stage.starting
    private(set) var joining = false
    var joinError: String?
    let store = Store(node: NodeClient(transport: ClientTransport()))
    private var running: Task<Void, Never>?

    /// The brand's accent: the build's until a device sends the space's.
    var accent: Color {
        Color(brandHex: store.brand?.accent ?? Bundle.main.object(forInfoDictionaryKey: "BrandAccent") as? String ?? "")
    }

    func start() async {
        guard stage == .starting else { return }
        let dir = URL.applicationSupportDirectory.appending(path: "client")
        do {
            stage = try await Client.start(in: dir) ? .joined : .welcome
        } catch {
            stage = .failed(error.localizedDescription)
        }
        if stage == .joined { run() }
    }

    /// Joins the space a pairing code is for, as `name`, and shows it.
    func join(_ code: String, name: String) async {
        guard !joining else { return }
        joining = true
        joinError = nil
        defer { joining = false }
        do {
            try await Client.join(code.trimmingCharacters(in: .whitespacesAndNewlines), name: name)
        } catch {
            joinError = error.localizedDescription
            return
        }
        stage = .joined
        run()
        try? await store.refreshState()
        // A device already in the space keeps the name it had there.
        if let current = store.state?.device.name, current != name { try? await store.renameDevice(name) }
    }

    /// `<cli>://join?code=…&name=…`: joins without the camera, from a link another device gives.
    func open(_ url: URL) {
        let query = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems ?? []
        let value = { (name: String) in query.first { $0.name == name }?.value?.trimmingCharacters(in: .whitespaces) }
        guard url.host() == "join", let code = value("code"), !code.isEmpty else { return }
        let name = value("name").flatMap { $0.isEmpty ? nil : $0 } ?? store.state?.device.name ?? UIDevice.current.name
        Task { await join(code, name: name) }
    }

    private func run() {
        guard running == nil else { return }
        running = Task { await store.run() }
    }
}

struct PhoneRoot: View {
    @Environment(Phone.self) private var phone

    var body: some View {
        switch phone.stage {
        case .starting:
            ProgressView()
        case .failed(let message):
            ContentUnavailableView("Can’t Start", systemImage: "exclamationmark.triangle", description: Text(message))
        case .welcome:
            WelcomeView()
        case .joined:
            HomeView()
        }
    }
}
