import SwiftUI

struct RootView: View {
    @Environment(Store.self) private var store

    var body: some View {
        @Bindable var store = store
        Group {
            if store.state == nil {
                if case .unreachable(let message) = store.phase {
                    ContentUnavailableView {
                        Label("Can’t Reach \(appName)", systemImage: "bolt.horizontal.circle")
                    } description: {
                        Text(message)
                    } actions: {
                        Text("It keeps trying.").foregroundStyle(.secondary)
                    }
                } else {
                    ProgressView("Starting \(appName)…")
                        .controlSize(.small)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            } else if store.install == nil {
                WelcomeView()
            } else {
                MainView()
            }
        }
        .sheet(item: $store.sheet) { sheet in
            switch sheet {
            case .pair: PairSheet()
            case .join: JoinSheet()
            case .pages: PagesEditor()
            }
        }
    }
}

private struct MainView: View {
    @Environment(Store.self) private var store

    var body: some View {
        NavigationSplitView {
            Sidebar()
                .navigationSplitViewColumnWidth(min: 200, ideal: 230, max: 320)
        } detail: {
            if store.install?.joining == true {
                ContentUnavailableView {
                    Label("Joining…", systemImage: "arrow.trianglehead.2.clockwise")
                } description: {
                    Text("This device is taking in what it keeps from the device it paired with.")
                }
            } else {
                @Bindable var store = store
                NavigationStack(path: $store.path) {
                    Group {
                        if let selection = store.selection { RouteView(route: selection, open: open) }
                    }
                    .navigationDestination(for: Route.self) { RouteView(route: $0, open: open) }
                }
                .id(store.selection)
            }
        }
    }

    /// An app opens in place of the page, selected in the sidebar; the shell's own views push.
    private func open(_ route: Route) {
        if case .screen = route { store.selection = route } else { store.path.append(route) }
    }
}

/// Creating an install, or joining one with a pairing code.
private struct WelcomeView: View {
    @Environment(Store.self) private var store
    @State private var name = ""
    @State private var you = NSFullUserName()
    @State private var error: String?
    @State private var recovery: String?

    private var ready: Bool { !name.trimmingCharacters(in: .whitespaces).isEmpty && !you.trimmingCharacters(in: .whitespaces).isEmpty }

    var body: some View {
        VStack(spacing: 28) {
            VStack(spacing: 14) {
                Image("Logo")
                    .resizable()
                    .frame(width: 112, height: 112)
                Text("Welcome to \(store.brand?.name ?? appName)")
                    .font(.largeTitle.weight(.semibold))
                Text("Your devices, and your team’s, as one place for apps and agents.\nStart it here, or join from another device.")
                    .font(.title3)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: 520)
            }
            VStack(spacing: 12) {
                Form {
                    TextField("Call it", text: $name, prompt: Text("The Van Tonders"))
                    TextField("Your name", text: $you)
                }
                .formStyle(.columns)
                .frame(width: 300)
                .onSubmit(create)
                Button("Start", action: create)
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                    .disabled(!ready)
                if let error {
                    Label(error, systemImage: "exclamationmark.circle.fill").foregroundStyle(.red)
                }
                Button("Have a pairing code? Join…") { store.sheet = .join }
                    .buttonStyle(.link)
            }
        }
        .padding(40)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .sheet(item: Binding(get: { recovery.map(RecoveryKey.init) }, set: { if $0 == nil { recovery = nil } })) { RecoverySheet(key: $0.key) }
    }

    private func create() {
        guard ready else { return }
        Task {
            do {
                var key: String?
                try await store.change { key = try await $0.create(name: name.trimmingCharacters(in: .whitespaces), userName: you.trimmingCharacters(in: .whitespaces)) }
                recovery = key
            } catch { self.error = error.localizedDescription }
        }
    }
}

private struct RecoveryKey: Identifiable {
    var key: String
    var id: String { key }
}

/// The recovery key, shown once, for 1Password: it restores the install from a backup when no server is left.
private struct RecoverySheet: View {
    @Environment(\.dismiss) private var dismiss
    var key: String

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Save Your Recovery Key").font(.title2.weight(.semibold))
            Text("Put it in 1Password now. It’s shown only this once, and it’s the only way to restore from a backup if every server is lost.")
                .foregroundStyle(.secondary)
            Text(key)
                .font(.system(.body, design: .monospaced))
                .textSelection(.enabled)
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(.quaternary.opacity(0.5), in: .rect(cornerRadius: 10))
            HStack {
                Button("Copy") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(key, forType: .string)
                }
                Spacer()
                Button("I’ve Saved It") { dismiss() }.keyboardShortcut(.defaultAction)
            }
        }
        .padding(24)
        .frame(width: 460)
    }
}
