import SwiftUI

/// What touching and holding an app's icon offers, and its ⋯ menu in Connected Apps: its settings, its page in the App
/// Store, taking it off this page, and deleting it, for its owners.
struct AppActions: View {
    @Environment(Store.self) private var store
    var app: AppInfo
    var page: Page
    var open: (Route) -> Void
    @Binding var deleting: AppInfo?
    @Binding var error: String?

    var body: some View {
        if let settings = store.settingsRoute(for: app, on: page) {
            Button("Settings", systemImage: "gearshape") { open(settings) }
        }
        if let route = store.storeRoute(for: app) {
            Button("Open in App Store", systemImage: "bag") { open(route) }
        }
        if store.canArrange(page.scope), (app.deploys ?? 0) > 0 {
            Button("Remove from \(page.title)", systemImage: "minus.circle", role: .destructive) {
                Task {
                    do { try await store.change { try await $0.install(app: app.name, scope: page.scope, false) } } catch { self.error = error.localizedDescription }
                }
            }
        }
        if store.owns(app), (app.deploys ?? 0) > 0 {
            Button("Delete App…", systemImage: "trash", role: .destructive) { deleting = app }
        }
    }
}

extension View {
    /// Asks before deleting `deleting`, then deletes it: it leaves every page and waits in Recently Deleted.
    func deleteAppDialog(_ deleting: Binding<AppInfo?>, error: Binding<String?>) -> some View {
        modifier(DeleteAppDialog(deleting: deleting, error: error))
    }
}

private struct DeleteAppDialog: ViewModifier {
    @Environment(Store.self) private var store
    @Binding var deleting: AppInfo?
    @Binding var error: String?

    func body(content: Content) -> some View {
        content
            .confirmationDialog("Delete \(deleting?.label ?? "")?", isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }),
                                titleVisibility: .visible, presenting: deleting) { app in
                Button("Delete App", role: .destructive) {
                    Task {
                        do { try await store.change { try await $0.retire(app: app.name, how: "deleted") } } catch { self.error = error.localizedDescription }
                    }
                }
            } message: { app in
                Text("\(app.label) leaves every page, for everyone, and stops running. It waits in the App Store’s Recently Deleted, with its data, until its owners erase it.")
            }
            .alert("Couldn’t Do That", isPresented: Binding(get: { error != nil }, set: { if !$0 { error = nil } })) {
                Button("OK") { error = nil }
            } message: {
                Text(error ?? "")
            }
    }
}

/// A page's Connected Apps folder, as an iOS folder's icon: its apps' icons, small, nine to a page, on frosted glass.
struct FolderTile: View {
    var apps: [AppInfo]
    var size: CGFloat

    var body: some View {
        let inset = size * 0.13, gap = size * 0.06
        let mini = (size - inset * 2 - gap * 2) / 3
        let shape = RoundedRectangle(cornerRadius: size * 0.225, style: .continuous)
        LazyVGrid(columns: Array(repeating: GridItem(.fixed(mini), spacing: gap), count: 3), alignment: .leading, spacing: gap) {
            ForEach(apps.prefix(9)) { app in
                AppTile(app: app, seed: app.name, size: mini)
            }
        }
        .padding(inset)
        .frame(width: size, height: size, alignment: .topLeading)
        .background(.regularMaterial, in: shape)
        .background(Color.secondary.opacity(0.18), in: shape)
        .overlay(shape.strokeBorder(.separator.opacity(0.5), lineWidth: 0.5))
        .accessibilityHidden(true)
    }
}

/// One account of a connected app, as its `accounts` gives it.
private struct ConnectedAccount: Decodable, Hashable, Identifiable {
    var account: String
    var ok: Bool
    var why: String?
    var id: String { account }
}

/// A page's Connected Apps, opened: a section per app, with its icon, title and ⋯ menu, then its accounts, each with a
/// dot saying whether it's signed in, then Add Account; and at the bottom, Add a Connected App. Swipe an account, or
/// press Edit, to sign it out.
struct ConnectedAppsView: View {
    @Environment(Store.self) private var store
    var page: Page
    var open: (Route) -> Void
    /// Each app's accounts, or why they couldn't be listed; none yet while loading.
    @State private var accounts: [String: Result<[ConnectedAccount], NodeError>] = [:]
    @State private var editing = false
    @State private var signingOut: (app: AppInfo, account: ConnectedAccount)?
    @State private var deleting: AppInfo?
    @State private var error: String?

    var body: some View {
        let apps = store.connectedApps(on: page)
        List {
            ForEach(apps) { app in
                Section {
                    accountRows(app)
                    if let add = store.settingsRoute(for: app, on: page, add: true) {
                        Button { open(add) } label: {
                            Label("Add Account", systemImage: "plus")
                        }
                    }
                } header: {
                    header(app)
                }
            }
            if apps.isEmpty {
                Text("No app on \(page.title) connects an account yet.").foregroundStyle(.secondary)
            }
            if store.canArrange(page.scope), let route = store.connectedStoreRoute {
                Section {
                    Button { open(route) } label: {
                        Label("Add a Connected App", systemImage: "plus.app")
                    }
                }
            }
        }
        #if os(iOS)
        .listStyle(.insetGrouped)
        #endif
        .navigationTitle("Connected Apps")
        .navigationSubtitle(subtitle(apps.count))
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button(editing ? "Done" : "Edit") { withAnimation { editing.toggle() } }
                    .disabled(accounts.values.allSatisfy { (try? $0.get())?.isEmpty ?? true })
            }
        }
        .task(id: apps.map(\.name)) { await load(apps) }
        .refreshable { await load(apps) }
        .onReceive(NotificationCenter.default.publisher(for: .nodeEvent)) { note in
            // An app's own change (a sign-in finished on its settings screen) lists its accounts again.
            guard let json = note.object as? String, let event = try? JSONDecoder().decode(NodeEvent.self, from: Data(json.utf8)),
                  event.type == "changed" else { return }
            let changed = apps.filter { event.apps?.contains($0.name) ?? true }
            Task { await load(changed) }
        }
        .confirmationDialog("Sign out of \(signingOut?.account.account ?? "")?", isPresented: Binding(get: { signingOut != nil }, set: { if !$0 { signingOut = nil } }),
                            titleVisibility: .visible, presenting: signingOut) { item in
            Button("Sign Out", role: .destructive) { Task { await signOut(item.app, item.account) } }
        } message: { item in
            Text("\(item.app.label) stops using it, here and for your agents. Signing in again takes you through \(item.app.label)’s sign-in.")
        }
        .deleteAppDialog($deleting, error: $error)
    }

    /// "For you only · 4 apps": whose page it is, and how many apps it connects.
    private func subtitle(_ count: Int) -> String {
        let whose = switch page.scope {
        case "user": String(localized: "For you only")
        case "global": String(localized: "For everyone")
        default: String(localized: "For \(page.title)")
        }
        return count == 1 ? String(localized: "\(whose) · 1 app") : String(localized: "\(whose) · \(count) apps")
    }

    private func header(_ app: AppInfo) -> some View {
        HStack(spacing: 10) {
            AppTile(app: app, seed: app.name, size: 28)
            Text(app.label)
                .font(.headline)
                .foregroundStyle(.primary)
                .textCase(nil)
            Spacer()
            Menu {
                AppActions(app: app, page: page, open: open, deleting: $deleting, error: $error)
            } label: {
                Image(systemName: "ellipsis.circle")
                    .font(.title3)
                    .accessibilityLabel("More for \(app.label)")
            }
            .menuStyle(.button)
            .buttonStyle(.borderless)
            .fixedSize()
        }
        .padding(.vertical, 4)
    }

    @ViewBuilder
    private func accountRows(_ app: AppInfo) -> some View {
        switch accounts[app.name] {
        case nil:
            HStack { ProgressView().controlSize(.small); Text("Loading…").foregroundStyle(.secondary) }
        case .failure(let failure):
            Label(failure.message, systemImage: "exclamationmark.triangle.fill")
                .font(.subheadline)
                .foregroundStyle(.orange)
        case .success(let list) where list.isEmpty:
            Text("No accounts yet").foregroundStyle(.secondary)
        case .success(let list):
            ForEach(list) { account in
                row(app, account)
                    .swipeActions(edge: .trailing) {
                        Button("Sign Out", role: .destructive) { signingOut = (app, account) }
                    }
            }
        }
    }

    private func row(_ app: AppInfo, _ account: ConnectedAccount) -> some View {
        HStack(spacing: 12) {
            if editing {
                Button { signingOut = (app, account) } label: {
                    Image(systemName: "minus.circle.fill").font(.title3).foregroundStyle(.white, .red)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Sign out of \(account.account)")
                .transition(.move(edge: .leading).combined(with: .opacity))
            }
            Circle()
                .fill(account.ok ? Color.green : Color.orange)
                .frame(width: 9, height: 9)
                .accessibilityLabel(account.ok ? "Signed in" : "Needs signing in again")
            VStack(alignment: .leading, spacing: 2) {
                Text(account.account).lineLimit(1).truncationMode(.middle)
                if !account.ok {
                    Text(NodeError(account.why ?? String(localized: "Needs signing in again")).message)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
            }
            Spacer(minLength: 8)
            if editing {
                Button("Sign Out", role: .destructive) { signingOut = (app, account) }
                    .buttonStyle(.borderless)
                    .foregroundStyle(.red)
            } else if !account.ok, let add = store.settingsRoute(for: app, on: page, add: true) {
                Button("Sign In") { open(add) }.buttonStyle(.borderless)
            }
        }
        .textSelection(.enabled)
    }

    private func load(_ apps: [AppInfo]) async {
        await withTaskGroup(of: (String, Result<[ConnectedAccount], NodeError>).self) { tasks in
            for app in apps {
                let node = store.node
                tasks.addTask {
                    do {
                        let data = try await node.call(app: app.name, op: "accounts", input: Data("{}".utf8))
                        return (app.name, .success(try JSONDecoder().decode([ConnectedAccount].self, from: data)))
                    } catch let failure as NodeError {
                        return (app.name, .failure(failure))
                    } catch is DecodingError {
                        return (app.name, .failure(NodeError("\(app.label) doesn’t list its accounts as connected apps do yet.")))
                    } catch {
                        return (app.name, .failure(NodeError(error.localizedDescription)))
                    }
                }
            }
            for await (name, result) in tasks { accounts[name] = result }
        }
    }

    private func signOut(_ app: AppInfo, _ account: ConnectedAccount) async {
        do {
            _ = try await store.node.call(app: app.name, op: "sign_out", input: try JSONEncoder().encode(["account": account.account]))
        } catch {
            self.error = error.localizedDescription
        }
        await load([app])
        await store.refreshWidgets()
    }
}
