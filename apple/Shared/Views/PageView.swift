import SwiftUI

#if os(macOS)
private let iconSize: CGFloat = 60
#else
@MainActor private let iconSize: CGFloat = UIDevice.current.userInterfaceIdiom == .pad ? 76 : 62
#endif

/// A page: its title (on a Mac; a phone's bar names it instead), then one grid, as the Home Screen's: its widgets in
/// their sizes, then its apps' icons, which flow around them, then the Connected Apps folder.
struct PageView: View {
    @Environment(Store.self) private var store
    var page: Page
    var open: (Route) -> Void
    @State private var width: CGFloat = 0
    @State private var deleting: AppInfo?
    @State private var error: String?

    var body: some View {
        let apps = store.apps(on: page), widgets = store.widgets(on: page), connected = store.connectedApps(on: page)
        let grid = GridMetrics(width: width)
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                #if os(macOS)
                PageHeader(title: page.title, subtitle: page.subtitle)
                #endif
                if apps.isEmpty && widgets.isEmpty && connected.isEmpty {
                    EmptyPage(page: page, open: open)
                } else if width > 0 {
                    HomeGrid(grid: grid) {
                        ForEach(widgets) { widget in
                            let size = grid.fit(store.size(of: widget, on: page))
                            PageWidget(widget: widget, page: page, size: size, grid: grid, open: open)
                                .layoutValue(key: GridSpan.self, value: WidgetSize.cells(size))
                        }
                        ForEach(apps) { app in
                            IconButton(title: app.label, badge: store.appBadge(app.name, on: page) ?? 0) { open(store.open(app, from: page)) } icon: {
                                AppTile(app: app, seed: app.name, size: grid.icon)
                            }
                            .contextMenu { AppActions(app: app, page: page, open: open, deleting: $deleting, error: $error) }
                        }
                        if !connected.isEmpty {
                            IconButton(title: String(localized: "Connected Apps")) { open(.connected(page.scope)) } icon: {
                                FolderTile(apps: connected, size: grid.icon)
                            }
                            .accessibilityLabel("Connected Apps, \(connected.count) apps")
                        }
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
            .padding(.horizontal, 20)
            #if os(iOS)
            .padding(.top, 20) // where the Home Screen's first row starts
            #endif
            .padding(.bottom, 96)
            .frame(maxWidth: 1100, alignment: .leading)
            .frame(maxWidth: .infinity)
        }
        .groupedBackground()
        .refreshable { try? await store.refreshState(); await store.refreshApps() }
        .deleteAppDialog($deleting, error: $error)
    }
}

/// A page's grid, as the Home Screen's: square cells, four across on a phone and more on wider screens, each holding
/// an icon; a widget takes 2×2 cells (small), 4×2 (medium), 4×4 (large) or 8×4 (xl).
struct GridMetrics {
    var columns: Int
    var cell: CGFloat
    var gap: CGFloat
    /// An icon's size, leaving room in its cell for its name.
    var icon: CGFloat

    @MainActor init(width: CGFloat) {
        if width < 500 {
            // A phone: four columns across the screen.
            gap = 12
            columns = 4
            cell = max(1, (width - gap * 3) / 4)
        } else {
            // Wider: cells stay an icon's size, so widgets keep the Home Screen's, and more of them fit across.
            gap = 14
            cell = iconSize + 22
            columns = max(4, Int((width + gap) / (cell + gap)))
        }
        icon = max(40, min(iconSize, cell - 19))
    }

    /// The points `n` cells span, with the gaps between them.
    func span(_ n: Int) -> CGFloat { CGFloat(n) * cell + CGFloat(n - 1) * gap }

    /// The sizes this grid has room for, of those a widget draws.
    func sizes(of widget: WidgetInfo) -> [String] { widget.supported.filter { WidgetSize.cells($0).w <= columns } }

    /// `size`, or large where an xl doesn't fit.
    func fit(_ size: String) -> String { WidgetSize.cells(size).w <= columns ? size : "large" }
}

/// How many cells across and down a view in a `HomeGrid` takes: one, unless it says.
struct GridSpan: LayoutValueKey {
    static let defaultValue = (w: 1, h: 1)
}

/// Lays its views out in a grid's cells in order, each in the first place it fits, row by row, so icons fill the
/// cells beside a small widget. Widgets start on an even column, as on the Home Screen.
struct HomeGrid: Layout {
    var grid: GridMetrics

    private func places(_ subviews: Subviews) -> (cells: [(x: Int, y: Int)], rows: Int) {
        var taken: [[Bool]] = []
        var cells: [(x: Int, y: Int)] = []
        for view in subviews {
            let span = view[GridSpan.self]
            let w = min(span.w, grid.columns), h = span.h
            var y = 0
            search: while true {
                for x in stride(from: 0, through: grid.columns - w, by: w > 1 ? 2 : 1) {
                    if (y..<y + h).allSatisfy({ row in row >= taken.count || (x..<x + w).allSatisfy { !taken[row][$0] } }) {
                        while taken.count < y + h { taken.append(Array(repeating: false, count: grid.columns)) }
                        for row in y..<y + h { for col in x..<x + w { taken[row][col] = true } }
                        cells.append((x, y))
                        break search
                    }
                }
                y += 1
            }
        }
        return (cells, taken.count)
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = places(subviews).rows
        return CGSize(width: grid.span(grid.columns), height: rows > 0 ? grid.span(rows) : 0)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let cells = places(subviews).cells
        for (view, cell) in zip(subviews, cells) {
            let span = view[GridSpan.self]
            let size = ProposedViewSize(width: grid.span(min(span.w, grid.columns)), height: grid.span(span.h))
            view.place(at: CGPoint(x: bounds.minX + CGFloat(cell.x) * (grid.cell + grid.gap), y: bounds.minY + CGFloat(cell.y) * (grid.cell + grid.gap)),
                       anchor: .topLeading, proposal: size)
        }
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

/// A widget on a page: tapping it opens where it links, or its app; touch and hold to size or hide it.
private struct PageWidget: View {
    @Environment(Store.self) private var store
    @Environment(\.openURL) private var openURL
    var widget: WidgetInfo
    var page: Page
    var size: String
    var grid: GridMetrics
    var open: (Route) -> Void

    var body: some View {
        let app = store.app(widget.app)
        WidgetCard(widget: widget, app: app, size: size)
            .environment(\.widgetHost, host)
            .onTapGesture { follow(widget.tree(size).link) }
            .contextMenu {
                let sizes = grid.sizes(of: widget)
                if sizes.count > 1 {
                    Picker("Size", selection: Binding(get: { size }, set: { new in withAnimation { store.setSize(new, of: widget, on: page) } })) {
                        ForEach(sizes, id: \.self) { Label(WidgetSize.title($0), systemImage: WidgetSize.symbol($0)).tag($0) }
                    }
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

/// A widget, filling the place it's given, as a Home Screen widget does: its tree at `size` under its title, small
/// and muted (unless the tree labels itself); a broken one says so, and why.
struct WidgetCard: View {
    var widget: WidgetInfo
    var app: AppInfo?
    var size = "medium"

    var body: some View {
        let tree = widget.tree(size)
        VStack(alignment: .leading, spacing: 6) {
            if tree.header != false || widget.error != nil {
                Text(widget.title ?? app?.label ?? widget.widget)
                    .font(.caption2.weight(.medium))
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
                    WidgetTree(node: tree, now: context.date)
                        .font(.footnote)
                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                }
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .clipped()
        .background(widgetBackground, in: .rect(cornerRadius: 22, style: .continuous))
        .contentShape(.rect(cornerRadius: 22, style: .continuous))
    }

    private var widgetBackground: some ShapeStyle {
        #if os(macOS)
        AnyShapeStyle(.fill.quaternary)
        #else
        AnyShapeStyle(Color(.secondarySystemGroupedBackground))
        #endif
    }
}

/// A widget card at `size`, as a phone's page draws it, for lists that show widgets outside a page's grid.
struct WidgetPreview: View {
    var widget: WidgetInfo
    var app: AppInfo?
    var size: String

    var body: some View {
        let grid = GridMetrics(width: 348)
        let cells = WidgetSize.cells(grid.fit(size))
        WidgetCard(widget: widget, app: app, size: grid.fit(size))
            .frame(width: grid.span(cells.w), height: grid.span(cells.h))
            .scaleEffect(0.85, anchor: .topLeading)
            .frame(width: grid.span(cells.w) * 0.85, height: grid.span(cells.h) * 0.85, alignment: .topLeading)
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
                                if widget.supported.count > 1 {
                                    Picker("Size", selection: Binding(get: { store.size(of: widget, on: page) }, set: { store.setSize($0, of: widget, on: page) })) {
                                        ForEach(widget.supported, id: \.self) { Text(WidgetSize.title($0)).tag($0) }
                                    }
                                    .pickerStyle(.segmented)
                                    .labelsHidden()
                                }
                                WidgetPreview(widget: widget, app: store.app(widget.app), size: store.size(of: widget, on: page))
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
                                    WidgetPreview(widget: widget, app: store.app(widget.app), size: widget.size ?? "medium").allowsHitTesting(false)
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
                        row(.install, value: "")
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
        case .install: .gray
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
