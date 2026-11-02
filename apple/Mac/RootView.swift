import AuthenticationServices
import SwiftUI

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
            } else if store.install == nil || !store.signedIn {
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
            case .vault(let scope, let id): VaultRequestSheet(scope: scope, id: id)
            case .ask(let ask): VaultRequestSheet(scope: ask.scope, id: "", ask: ask)
            case .addWidget(let scope): AddWidgetSheet(scope: scope)
            case .services(let scope): ServicesSheet(scope: scope)
            }
        }
        .onChange(of: appearance, initial: true) {
            NSApp.appearance = Appearance(rawValue: appearance)?.scheme.map { NSAppearance(named: $0 == .dark ? .darkAqua : .aqua)! }
        }
    }
}

private struct MainView: View {
    @Environment(Mac.self) private var mac
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
            .safeAreaInset(edge: .bottom) { if mac.asMachine { SignInBar() } }
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

    /// An app, or a Settings screen, opens in place of the page, selected in the sidebar; a page's Services, as a
    /// sheet over it.
    private func open(_ route: Route) {
        switch route {
        case .screen, .setting: store.selection = route
        case .services(let scope): store.sheet = .services(scope)
        default: store.path.append(route)
        }
    }
}

/// A window that showed pages before it could sign in, asking its person to: at the address this Mac's node is in.
private struct SignInBar: View {
    @Environment(Mac.self) private var mac
    @Environment(Store.self) private var store
    @Environment(\.webAuthenticationSession) private var session

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if !store.sidebarFolded {
                Text("Sign in with your passkey, as on your other devices.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                if let error = mac.signInError {
                    Text(error).font(.caption).foregroundStyle(.red).fixedSize(horizontal: false, vertical: true)
                }
            }
            Button {
                guard let address = store.install?.settings?["address"] else { return }
                Task { _ = await mac.signIn(address, session: session) }
            } label: {
                if store.sidebarFolded {
                    Image(systemName: "person.badge.key")
                } else {
                    Text("Sign In").frame(maxWidth: .infinity)
                }
            }
            .help("Sign In")
            .buttonStyle(.borderedProminent)
            .disabled(mac.signingIn || store.install?.settings?["address"] == nil)
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.bar)
    }
}

/// A page's Services, over the page: what it opens (a settings screen, the App Store) opens in the window.
private struct ServicesSheet: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    var scope: String

    var body: some View {
        NavigationStack {
            if let page = store.page(scope) {
                ServicesView(page: page) { route in
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

/// Signed out: your One App's address, or an invite link to it, then your passkey, as on the iPhone. A Mac whose node
/// is in no install yet joins one first, with a pairing code from another of your computers.
private struct WelcomeView: View {
    @Environment(Mac.self) private var mac
    @Environment(Store.self) private var store
    @Environment(\.webAuthenticationSession) private var session
    @Environment(\.openURL) private var openURL
    @State private var address = ""
    @State private var invited = false
    @State private var asking = false

    private var given: String { address.trimmingCharacters(in: .whitespacesAndNewlines) }

    /// "your One App", or a workplace's, "the PTC App" (project.sh).
    private var yours: String { Bundle.main.object(forInfoDictionaryKey: "Yours") as? String ?? "" }
    private var whose: String { yours.prefix(1).uppercased() + yours.dropFirst() }

    var body: some View {
        VStack(spacing: 28) {
            VStack(spacing: 14) {
                Image("Logo")
                    .resizable()
                    .frame(width: 112, height: 112)
                    .accessibilityHidden(true)
                Text(appName)
                    .font(.largeTitle.weight(.semibold))
                if invited {
                    Text("Made your passkey? Sign in with it.")
                        .font(.title3)
                        .foregroundStyle(.secondary)
                }
            }
            .multilineTextAlignment(.center)
            if store.install == nil {
                join
            } else {
                signIn
            }
        }
        .padding(40)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .animation(.snappy, value: mac.signInError)
        .sheet(isPresented: $asking) { AskYourAgentSheet(prompt: agentPrompt(address: given.isEmpty ? nil : given)) }
        .onAppear {
            // The address this Mac's node is in, unless the person gave another.
            let known = Auth.address ?? store.install?.settings?["address"].flatMap(URL.init(string:))
            if address.isEmpty, let known { address = Self.shown(known) }
        }
    }

    private var signIn: some View {
        VStack(spacing: 14) {
            VStack(alignment: .leading, spacing: 6) {
                TextField("Address", text: $address, prompt: Text("one.example.com"))
                    .textContentType(.URL)
                    .autocorrectionDisabled()
                    .textFieldStyle(.roundedBorder)
                    .controlSize(.large)
                    .onSubmit(go)
                Text("\(whose)’s address, or the invite link you were sent.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            .frame(width: 340)
            if let error = mac.signInError {
                Label(error, systemImage: "exclamationmark.circle.fill")
                    .foregroundStyle(.red)
                    .frame(maxWidth: 420)
            }
            Button(action: go) {
                Group {
                    if mac.signingIn {
                        ProgressView().controlSize(.small)
                    } else {
                        Text(given.contains("/invite#") ? "Open Invite" : "Sign In")
                    }
                }
                .frame(width: 120)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .keyboardShortcut(.defaultAction)
            .disabled(given.isEmpty || mac.signingIn)
            Button("Or Ask Your Agent") { asking = true }
                .buttonStyle(.link)
        }
    }

    private var join: some View {
        VStack(spacing: 14) {
            Text("This Mac isn’t part of \(yours) yet. Join it from another of your computers, with a pairing code.")
                .font(.title3)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 460)
            Button("Join with a Pairing Code…") { store.sheet = .join }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
            Button("Or Ask Your Agent") { asking = true }
                .buttonStyle(.link)
        }
    }

    /// An invite link opens in the browser, where the person makes their passkey; then they sign in with it here.
    private func go() {
        guard !given.isEmpty else { return }
        Task {
            if let invite = await mac.signIn(given, session: session) {
                invited = true
                address = Auth.address.map(Self.shown) ?? ""
                openURL(invite)
            }
        }
    }

    private static func shown(_ url: URL) -> String {
        url.absoluteString.replacingOccurrences(of: "https://", with: "").trimmingCharacters(in: CharacterSet(charactersIn: "/"))
    }
}
