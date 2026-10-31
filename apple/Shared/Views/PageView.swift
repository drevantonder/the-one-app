import SwiftUI

#if os(macOS)
private let iconSize: CGFloat = 60
#else
@MainActor private let iconSize: CGFloat = UIDevice.current.userInterfaceIdiom == .pad ? 76 : 62
#endif

/// A page: its title (on a Mac; a phone's bar names it instead), the widgets of the apps installed in its scope, then their icons. Wide, the widgets take
/// a column on the left and the icons the rest.
struct PageView: View {
    @Environment(Store.self) private var store
    var page: Page
    var open: (Route) -> Void
    @State private var width: CGFloat = 0

    var body: some View {
        let apps = store.apps(on: page), widgets = store.widgets(on: page), connected = store.connectedApps(on: page)
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                #if os(macOS)
                PageHeader(title: page.title, subtitle: page.subtitle)
                #endif
                if apps.isEmpty && widgets.isEmpty && connected.isEmpty {
                    EmptyPage(page: page, open: open)
                } else if width > 700 {
                    HStack(alignment: .top, spacing: 28) {
                        if !widgets.isEmpty {
                            WidgetColumn(widgets: widgets, page: page, open: open).frame(width: min(360, width * 0.4))
                        }
                        IconGrid(apps: apps, connected: connected, page: page, open: open)
                    }
                } else {
                    WidgetColumn(widgets: widgets, page: page, open: open)
                    IconGrid(apps: apps, connected: connected, page: page, open: open)
                }
            }
            .padding(.horizontal, 20)
            #if os(iOS)
            .padding(.top, 20) // where the Home Screen's first row starts
            #endif
            .padding(.bottom, 96)
            .frame(maxWidth: 1100, alignment: .leading)
            .frame(maxWidth: .infinity)
        }
        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
        .groupedBackground()
        .refreshable { try? await store.refreshState(); await store.refreshApps() }
    }
}

/// A page's title, large at its top. On a Mac it is the window's title too, though the toolbar doesn't repeat it.
struct PageHeader: View {
    var title: String
    var subtitle: String

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title)
                .font(.largeTitle.bold())
                .foregroundStyle(.primary)
            Text(subtitle)
                .font(.subheadline)
                .foregroundStyle(.secondary)
        }
        .padding(.top, 4)
        .padding(.horizontal, 4)
        #if os(macOS)
        .navigationTitle(title)
        .navigationSubtitle(subtitle)
        .toolbar(removing: .title)
        #endif
    }
}

/// A page's widgets, each as wide as the column but for small ones, which go two to a row.
private struct WidgetColumn: View {
    @Environment(Store.self) private var store
    var widgets: [WidgetInfo]
    var page: Page
    var open: (Route) -> Void

    var body: some View {
        VStack(spacing: 12) {
            ForEach(rows, id: \.first!.id) { row in
                HStack(alignment: .top, spacing: 12) {
                    ForEach(row) { widget in
                        PageWidget(widget: widget, page: page, size: store.size(of: widget, on: page), open: open)
                    }
                    if row.count == 1, store.size(of: row[0], on: page) == "small" { Color.clear.frame(maxWidth: .infinity) }
                }
            }
        }
    }

    /// Widgets in rows: small ones in pairs, in order.
    private var rows: [[WidgetInfo]] {
        var rows: [[WidgetInfo]] = []
        for widget in widgets {
            if store.size(of: widget, on: page) == "small", let last = rows.last, last.count == 1, store.size(of: last[0], on: page) == "small" {
                rows[rows.count - 1].append(widget)
            } else {
                rows.append([widget])
            }
        }
        return rows
    }
}

/// A widget on a page: tapping it opens where it links, or its app; touch and hold to size or hide it.
private struct PageWidget: View {
    @Environment(Store.self) private var store
    @Environment(\.openURL) private var openURL
    var widget: WidgetInfo
    var page: Page
    var size: String
    var open: (Route) -> Void

    var body: some View {
        let app = store.app(widget.app)
        WidgetCard(widget: widget, app: app, size: size)
            .environment(\.widgetHost, host)
            .onTapGesture { follow(widget.root.link) }
            .contextMenu {
                Picker("Size", selection: Binding(get: { size }, set: { store.setSize($0, of: widget, on: page) })) {
                    Label("Small", systemImage: "square").tag("small")
                    Label("Medium", systemImage: "rectangle").tag("medium")
                    Label("Large", systemImage: "rectangle.portrait").tag("large")
                }
                Button("Hide from \(page.title)", systemImage: "eye.slash") { withAnimation { store.setHidden(widget, on: page, true) } }
                Button("Edit Widgets…", systemImage: "square.grid.2x2") { store.sheet = .widgets(page.scope) }
                if let error = widget.error {
                    Button("Copy What Went Wrong", systemImage: "doc.on.doc") { copy(error) }
                }
            }
    }

    private var host: WidgetHost {
        WidgetHost(app: widget.app, scope: page.scope, run: { [store] op, input in await store.run(app: widget.app, op: op, input: input) },
                   open: { follow($0) })
    }

    /// A link: a web page in the browser, a screen pushed; none, the app's first screen.
    private func follow(_ link: WidgetLink?) {
        guard let link else {
            if let app = store.app(widget.app), app.hasScreens { open(store.open(app, from: page)) }
            return
        }
        if case .url(let url) = link { openURL(url) } else if let route = store.route(for: link, app: widget.app, scope: page.scope) { open(route) }
    }
}

func copy(_ text: String) {
    #if os(macOS)
    NSPasteboard.general.clearContents()
    NSPasteboard.general.setString(text, forType: .string)
    #else
    UIPasteboard.general.string = text
    #endif
}

/// A widget in a grouped container, under its app's icon and title; a broken one says so, and why.
struct WidgetCard: View {
    var widget: WidgetInfo
    var app: AppInfo?
    var size = "medium"

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 7) {
                AppTile(app: app, seed: widget.app, size: 18)
                Text(widget.title ?? app?.label ?? widget.widget)
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            if let error = widget.error {
                VStack(alignment: .leading, spacing: 4) {
                    Label("This widget stopped working.", systemImage: "exclamationmark.triangle.fill")
                        .font(.subheadline.weight(.medium))
                        .foregroundStyle(.orange)
                    Text(NodeError(error).message)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(3)
                        .textSelection(.enabled)
                    Text("Touch and hold to copy it for \(app?.label ?? widget.app)’s maker.")
                        .font(.caption)
                        .foregroundStyle(.tertiary)
                }
            } else {
                TimelineView(.everyMinute) { context in
                    WidgetTree(node: widget.root, now: context.date)
                        .font(.subheadline)
                }
            }
            if size != "medium" { Spacer(minLength: 0) }
        }
        .padding(16)
        .frame(maxWidth: .infinity, minHeight: size == "large" ? 260 : size == "small" ? 140 : nil, alignment: .topLeading)
        .groupedContainer()
        .contentShape(.rect)
    }
}

/// Every widget there is: this page's, to show, hide and size, and the other pages', which come with their apps.
struct WidgetsEditor: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    var scope: String

    var body: some View {
        let page = store.page(scope)
        NavigationStack {
            List {
                if let page {
                    let here = store.widgets(on: page, hidden: true)
                    Section {
                        ForEach(here) { widget in
                            VStack(alignment: .leading, spacing: 10) {
                                Toggle(isOn: Binding(get: { !store.hiddenWidgets.contains("\(scope)/\(widget.id)") },
                                                     set: { store.setHidden(widget, on: page, !$0) })) {
                                    Text(widget.title ?? store.app(widget.app)?.label ?? widget.widget)
                                }
                                Picker("Size", selection: Binding(get: { store.size(of: widget, on: page) }, set: { store.setSize($0, of: widget, on: page) })) {
                                    Text("Small").tag("small")
                                    Text("Medium").tag("medium")
                                    Text("Large").tag("large")
                                }
                                .pickerStyle(.segmented)
                                .labelsHidden()
                                WidgetCard(widget: widget, app: store.app(widget.app), size: store.size(of: widget, on: page))
                                    .allowsHitTesting(false)
                            }
                            .padding(.vertical, 4)
                        }
                        if here.isEmpty { Text("No app on \(page.title) has a widget.").foregroundStyle(.secondary) }
                    } header: {
                        Text("On \(page.title)")
                    } footer: {
                        Text("Switch one off to hide it from this page on this device.")
                    }
                    let elsewhere = elsewhere(than: page)
                    if !elsewhere.isEmpty {
                        Section {
                            ForEach(elsewhere, id: \.0.id) { widget, other in
                                VStack(alignment: .leading, spacing: 6) {
                                    Text("On \(other.title)").font(.caption).foregroundStyle(.secondary)
                                    WidgetCard(widget: widget, app: store.app(widget.app)).allowsHitTesting(false)
                                }
                                .padding(.vertical, 4)
                            }
                        } header: {
                            Text("More Widgets")
                        } footer: {
                            Text(store.canArrange(scope)
                                 ? "A widget comes with its app: add the app to \(page.title) from the App Store to have it here."
                                 : "A widget comes with its app, which \(page.title)’s admins add.")
                        }
                    }
                }
            }
            .navigationTitle("Widgets")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
            }
        }
        #if os(macOS)
        .frame(width: 440, height: 560)
        #endif
    }

    /// The widgets of other pages whose apps aren't on this one, once each.
    private func elsewhere(than page: Page) -> [(WidgetInfo, Page)] {
        let here = Set(store.widgets(on: page, hidden: true).map(\.id))
        var seen = here
        return store.allPages.filter { $0 != page }.flatMap { other in
            store.widgets(on: other, hidden: true).filter { seen.insert($0.id).inserted }.map { ($0, other) }
        }
    }
}

/// A page's icons, then its Connected Apps folder, if any of its apps only connects an account.
private struct IconGrid: View {
    @Environment(Store.self) private var store
    var apps: [AppInfo]
    var connected: [AppInfo]
    var page: Page
    var open: (Route) -> Void
    @State private var deleting: AppInfo?
    @State private var error: String?

    var body: some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: iconSize + 18, maximum: iconSize + 34), spacing: 10, alignment: .top)],
                  alignment: .leading, spacing: 20) {
            ForEach(apps) { app in
                IconButton(title: app.label, badge: store.appBadge(app.name, on: page) ?? 0) { open(store.open(app, from: page)) } icon: {
                    AppTile(app: app, seed: app.name, size: iconSize)
                }
                    .contextMenu { AppActions(app: app, page: page, open: open, deleting: $deleting, error: $error) }
            }
            if !connected.isEmpty {
                IconButton(title: String(localized: "Connected Apps")) { open(.connected(page.scope)) } icon: {
                    FolderTile(apps: connected, size: iconSize)
                }
                .accessibilityLabel("Connected Apps, \(connected.count) apps")
            }
        }
        .deleteAppDialog($deleting, error: $error)
    }
}

/// A large rounded icon with its name under it, as a home screen's, with a count in a red badge as its icons have.
struct IconButton<Icon: View>: View {
    var title: String
    var badge = 0
    var action: () -> Void
    @ViewBuilder var icon: Icon

    var body: some View {
        Button(action: action) {
            VStack(spacing: 6) {
                icon
                    .overlay(alignment: .topTrailing) {
                        if badge > 0 {
                            Text(badge > 99 ? "99+" : "\(badge)")
                                .font(.caption.weight(.semibold).monospacedDigit())
                                .foregroundStyle(.white)
                                .padding(.horizontal, 6)
                                .frame(minWidth: 22, minHeight: 22)
                                .background(.red, in: .capsule)
                                .offset(x: 7, y: -7)
                                .accessibilityLabel("\(badge) waiting")
                        }
                    }
                Text(title)
                    .font(.caption)
                    .lineLimit(1)
                    .foregroundStyle(.primary)
            }
            .frame(maxWidth: .infinity)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
    }
}

private struct EmptyPage: View {
    @Environment(Store.self) private var store
    var page: Page
    var open: (Route) -> Void

    var body: some View {
        VStack(spacing: 10) {
            Text("Nothing here yet")
                .font(.headline)
            Text(store.canArrange(page.scope)
                 ? "Apps you add to \(page.title) show up here, with their widgets above them."
                 : "When its admins add apps to \(page.title), they show up here.")
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            if store.canArrange(page.scope) {
                Button("Open the App Store") { open(.setting(.store)) }
                    .buttonStyle(.borderedProminent)
                    .padding(.top, 4)
            }
        }
        .frame(maxWidth: .infinity)
        .padding(28)
        .groupedContainer()
    }
}

/// The cog's page: who you are, then the shell's own views in groups, each with its count or value, then this
/// build. A phone and an iPad show it as the last page.
struct SettingsPage: View {
    @Environment(Store.self) private var store
    var open: (Route) -> Void
    @AppStorage("appearance") private var appearance = Appearance.automatic.rawValue
    @State private var secrets: Int?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                #if os(macOS)
                PageHeader(title: String(localized: "Settings"), subtitle: store.install?.name ?? "")
                #endif
                Profile()
                if store.requests > 0 {
                    SettingsSection {
                        row(.requests, value: "\(store.requests)", badge: true)
                    }
                }
                SettingsSection {
                    row(.store, value: appsValue)
                    row(.groups, value: count(myGroups))
                    row(.devices, value: count(myDevices))
                }
                SettingsSection {
                    row(.secrets, value: secrets.map { $0 == 0 ? String(localized: "None") : count($0) } ?? "")
                    row(.access, value: (store.state?.opens.isEmpty ?? true) ? String(localized: "None") : count(store.state?.opens.count ?? 0))
                    row(.appearance, value: (Appearance(rawValue: appearance) ?? .automatic).title)
                }
                if store.isAdmin {
                    SettingsSection(footer: String(localized: "Only admins see these.")) {
                        row(.users, value: count(store.install?.users.count ?? 0))
                        row(.backups, value: "")
                    }
                }
                Text(version)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity)
                    .multilineTextAlignment(.center)
            }
            .padding(.horizontal, 20)
            #if os(iOS)
            .padding(.top, 20) // where the Home Screen's first row starts
            #endif
            .padding(.bottom, 96)
            .frame(maxWidth: 700, alignment: .leading)
            .frame(maxWidth: .infinity)
        }
        .groupedBackground()
        .task { secrets = try? await store.node.secrets(scope: "user").count }
    }

    private func row(_ item: Setting, value: String, badge: Bool = false) -> some View {
        SettingsRow(item: item, value: value, badge: badge) { open(.setting(item)) }
    }

    private func count(_ n: Int) -> String { n.formatted() }

    private var myGroups: Int {
        guard let me = store.me else { return 0 }
        return store.install?.groups.filter { $0.role(of: me.id) != nil }.count ?? 0
    }

    private var myDevices: Int {
        guard let me = store.me, let install = store.install else { return 0 }
        return install.servers.filter { $0.user == me.id }.count + install.clients.filter { $0.user == me.id }.count
    }

    /// "4 of 9": the apps on your pages, of all there are.
    private var appsValue: String {
        let all = store.apps.count, mine = store.apps.filter { !$0.scopes.isEmpty }.count
        return all == 0 ? "" : String(localized: "\(mine) of \(all)")
    }

    /// The app's version and build, and the node's, for telling which build a device runs.
    private var version: String {
        let info = Bundle.main.infoDictionary ?? [:]
        let app = "\(store.brand?.name ?? appName), app \(info["BuildCommit"] as? String ?? "")"
        let host = store.state?.host ?? store.state?.machine
        let server = store.install?.servers.first { $0.key == host?.key }
        guard let build = server?.build else { return app }
        return "\(app)\n\(displayName(server?.name ?? "")) runs \(String(build.prefix(12)))"
    }
}

/// Who you are, at the top of Settings: your initials, name, role and install, and this device.
private struct Profile: View {
    @Environment(Store.self) private var store

    var body: some View {
        if let me = store.me {
            HStack(spacing: 14) {
                Avatar(id: me.id, name: me.name, size: 60)
                VStack(alignment: .leading, spacing: 3) {
                    Text(me.name)
                        .font(.title2.weight(.semibold))
                        .lineLimit(2)
                        .minimumScaleFactor(0.8)
                    Text(me.admin ? "Admin of \(store.install?.name ?? "")" : "Member of \(store.install?.name ?? "")")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                    if let machine = store.state?.machine.name {
                        Label(displayName(machine), systemImage: deviceSymbol(machine))
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                }
                .textSelection(.enabled)
                Spacer(minLength: 0)
            }
            .padding(16)
            .groupedContainer()
            .accessibilityElement(children: .combine)
        }
    }
}

/// Rows in a grouped container, as an inset list's section draws them, with dividers between.
struct SettingsSection<Content: View>: View {
    var footer: String?
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            VStack(spacing: 0) {
                Group(subviews: content) { rows in
                    ForEach(Array(rows.enumerated()), id: \.offset) { i, row in
                        if i > 0 { Divider().padding(.leading, 58) }
                        row
                    }
                }
            }
            .groupedContainer()
            if let footer {
                Text(footer).font(.footnote).foregroundStyle(.secondary).padding(.horizontal, 16)
            }
        }
    }
}

/// A row of Settings: its icon, its name, its value, and a chevron.
private struct SettingsRow: View {
    var item: Setting
    var value: String
    var badge = false
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 14) {
                SymbolTile(symbol: item.symbol, color: item.color, size: 30)
                Text(item.title)
                    .foregroundStyle(.primary)
                Spacer(minLength: 8)
                if badge {
                    Text(value)
                        .font(.callout.weight(.semibold))
                        .foregroundStyle(.white)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 2)
                        .background(.red, in: .capsule)
                } else {
                    Text(value)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                Image(systemName: "chevron.right")
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(.tertiary)
            }
            .padding(.horizontal, 14)
            .frame(minHeight: 50)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityValue(value)
    }
}

/// Light, Dark or Automatic, as small pictures of a screen with a check under each, as iOS's Display settings has.
struct AppearanceView: View {
    @AppStorage("appearance") private var appearance = Appearance.automatic.rawValue

    var body: some View {
        Form {
            Section {
                HStack(spacing: 0) {
                    ForEach(Appearance.allCases) { option in
                        Button { appearance = option.rawValue } label: {
                            VStack(spacing: 10) {
                                Thumbnail(option: option)
                                Text(option.title).font(.subheadline).foregroundStyle(.primary)
                                Image(systemName: appearance == option.rawValue ? "checkmark.circle.fill" : "circle")
                                    .font(.title3)
                                    .foregroundStyle(appearance == option.rawValue ? AnyShapeStyle(.tint) : AnyShapeStyle(.tertiary))
                            }
                            .frame(maxWidth: .infinity)
                            .contentShape(.rect)
                        }
                        .buttonStyle(.plain)
                        .accessibilityAddTraits(appearance == option.rawValue ? .isSelected : [])
                    }
                }
                .padding(.vertical, 10)
            } footer: {
                Text("Automatic follows this device’s own setting. App screens follow it too.")
            }
        }
        .formStyle(.grouped)
        .navigationTitle("Appearance")
    }

    private struct Thumbnail: View {
        var option: Appearance

        var body: some View {
            let shape = RoundedRectangle(cornerRadius: 10, style: .continuous)
            ZStack {
                switch option {
                case .light: screen(dark: false)
                case .dark: screen(dark: true)
                case .automatic:
                    screen(dark: false).overlay(alignment: .trailing) { screen(dark: true).frame(width: 33).clipped() }
                }
            }
            .frame(width: 66, height: 120)
            .clipShape(shape)
            .overlay(shape.strokeBorder(.separator))
        }

        private func screen(dark: Bool) -> some View {
            let fg = dark ? Color.white : Color.black
            return (dark ? Color.black : Color.white).overlay(alignment: .topLeading) {
                VStack(alignment: .leading, spacing: 6) {
                    Capsule().fill(fg.opacity(0.7)).frame(width: 30, height: 5)
                    ForEach(0..<4, id: \.self) { _ in
                        RoundedRectangle(cornerRadius: 3).fill(fg.opacity(0.12)).frame(width: 50, height: 14)
                    }
                }
                .padding(8)
            }
            .frame(width: 66, height: 120, alignment: .leading)
        }
    }
}

extension Setting {
    /// Its icon's colour, as Settings colours its own.
    var color: Color {
        switch self {
        case .store: .blue
        case .groups: .green
        case .devices: .gray
        case .secrets: .indigo
        case .access: .purple
        case .users: .teal
        case .backups: .orange
        case .requests: .pink
        case .appearance: .blue
        }
    }
}

extension AppTile {
    /// `app`'s tile; `seed` names it when it's not known here.
    init(app: AppInfo?, seed: String, size: CGFloat) {
        self.init(icon: app?.icon, seed: app?.name ?? seed, color: app?.color, image: app?.image, size: size)
    }
}
