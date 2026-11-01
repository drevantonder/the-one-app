import SwiftUI
import AuthenticationServices

struct RootView: View {
    @Environment(Store.self) private var store
    @AppStorage("appearance") private var appearance = Appearance.automatic.rawValue

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
            case .secret(let scope, let id): SecretRequestSheet(scope: scope, id: id)
            case .widgets(let scope): WidgetsEditor(scope: scope)
            case .connectors(let scope): ConnectorsSheet(scope: scope)
            }
        }
        .onChange(of: appearance, initial: true) {
            NSApp.appearance = Appearance(rawValue: appearance)?.scheme.map { NSAppearance(named: $0 == .dark ? .darkAqua : .aqua)! }
        }
    }
}

private struct MainView: View {
    @Environment(Store.self) private var store

    var body: some View {
        NavigationSplitView {
            Group {
                if store.sidebarFolded {
                    Sidebar().navigationSplitViewColumnWidth(64)
                } else {
                    Sidebar().navigationSplitViewColumnWidth(min: 200, ideal: 230, max: 320)
                }
            }
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

    /// An app opens in place of the page, selected in the sidebar; a page's Connectors, as a sheet over it; the
    /// shell's own views push.
    private func open(_ route: Route) {
        switch route {
        case .screen: store.selection = route
        case .connectors(let scope): store.sheet = .connectors(scope)
        default: store.path.append(route)
        }
    }
}

/// A page's Connectors, over the page: what it opens (a settings screen, the App Store) opens in the window.
private struct ConnectorsSheet: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    var scope: String

    var body: some View {
        NavigationStack {
            if let page = store.page(scope) {
                ConnectorsView(page: page) { route in
                    dismiss()
                    if case .screen = route { store.selection = route } else { store.path.append(route) }
                }
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) { Button("Done") { dismiss() } }
                }
            }
        }
        .frame(width: 460, height: 560)
    }
}

/// Creating an install, or joining one with a pairing code; or, for an edition with one install, signing in to it.
private struct WelcomeView: View {
    @Environment(Store.self) private var store
    @Environment(\.webAuthenticationSession) private var session
    @State private var name = ""
    @State private var you = NSFullUserName()
    @State private var error: String?
    @State private var recovery: String?
    @State private var signingIn = false

    private var ready: Bool { !name.trimmingCharacters(in: .whitespaces).isEmpty && !you.trimmingCharacters(in: .whitespaces).isEmpty }

    var body: some View {
        VStack(spacing: 28) {
            VStack(spacing: 14) {
                Image("Logo")
                    .resizable()
                    .frame(width: 112, height: 112)
                Text("Welcome to \(store.config?.name ?? appName)")
                    .font(.largeTitle.weight(.semibold))
                Group {
                    if SignIn.configured == nil {
                        Text("Your devices, and your team’s, as one place for apps and agents.\nStart it here, or join from another device.")
                    } else {
                        Text("Your devices, and your team’s, as one place for apps and agents.")
                    }
                }
                .font(.title3)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 520)
            }
            if let signIn = SignIn.configured { signInForm(signIn) } else { createForm }
        }
        .padding(40)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .sheet(item: Binding(get: { recovery.map(RecoveryKey.init) }, set: { if $0 == nil { recovery = nil } })) { RecoverySheet(key: $0.key) }
    }

    private func signInForm(_ signIn: SignIn) -> some View {
        VStack(spacing: 12) {
            Button { start(signIn) } label: {
                Text(signIn.title)
                    .opacity(signingIn ? 0 : 1)
                    .overlay { if signingIn { ProgressView().controlSize(.small) } }
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .disabled(signingIn)
            if let error {
                Label(error, systemImage: "exclamationmark.circle.fill")
                    .foregroundStyle(.red)
                    .frame(maxWidth: 440)
            }
            Button("Have a pairing code? Join…") { store.sheet = .join }
                .buttonStyle(.link)
        }
    }

    private func start(_ signIn: SignIn) {
        Task {
            signingIn = true
            error = nil
            defer { signingIn = false }
            do {
                guard let (code, _) = try await signIn.run(session) else { return }
                try await store.join(code)
            } catch let failure as SignInError {
                error = failure.message
            } catch {
                self.error = joinFailure(error)
            }
        }
    }

    private var createForm: some View {
        VStack(spacing: 12) {
            Form {
                TextField("Call it", text: $name, prompt: Text("Your family or team"))
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
