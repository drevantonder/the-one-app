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

    func join(_ ticket: String, name: String) async throws {
        try await Client.join(ticket.trimmingCharacters(in: .whitespacesAndNewlines), name: name)
        stage = .joined
        run()
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
