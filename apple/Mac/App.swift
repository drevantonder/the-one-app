import ServiceManagement
import SwiftUI

@main
struct MacApp: App {
    @State private var store: Store

    init() {
        let store = Store(node: .local)
        store.startNode = NodeLauncher.start
        store.preferredDeviceName = Host.current().localizedName
        Task { await store.run() }
        _store = State(initialValue: store)
        if !UserDefaults.standard.bool(forKey: "loginItemSet") {
            UserDefaults.standard.set(true, forKey: "loginItemSet")
            try? SMAppService.mainApp.register()
        }
    }

    var body: some Scene {
        Window(appName, id: "main") {
            RootView()
                .environment(store)
                .tint(accent)
                .frame(minWidth: 720, minHeight: 460)
        }
        .defaultSize(width: 1000, height: 700)
        .commands {
            CommandGroup(replacing: .newItem) {
                Button("New Topic…") { store.sheet = .newTopic(store.topic ?? "") }
                    .keyboardShortcut("n")
                    .disabled(store.space == nil)
                Button("New Space…") { store.sheet = .newSpace }
                    .keyboardShortcut("n", modifiers: [.command, .shift])
                Divider()
                Button("Invite a Device…") { store.sheet = .invite }
                    .keyboardShortcut("i", modifiers: [.command, .shift])
                    .disabled(store.space == nil)
                Button("Join a Space…") { store.sheet = .join }
                    .keyboardShortcut("j", modifiers: [.command, .shift])
            }
        }

        MenuBarExtra {
            MenuPanel()
                .environment(store)
                .tint(accent)
        } label: {
            MenuLabel(store: store)
        }
        .menuBarExtraStyle(.window)
    }

    private var accent: Color {
        store.brand.map { Color(brandHex: $0.accent) } ?? .accentColor
    }
}

/// Keeps this device's node running: a login-persistent launch agent, or a child process if that's refused.
@MainActor
enum NodeLauncher {
    static var cli: String { Bundle.main.object(forInfoDictionaryKey: "NodeCLI") as? String ?? "" }
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
