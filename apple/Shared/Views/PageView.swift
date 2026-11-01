import SwiftUI

#if os(macOS)
private let iconSize: CGFloat = 60
#else
@MainActor private let iconSize: CGFloat = UIDevice.current.userInterfaceIdiom == .pad ? 76 : 62
#endif

/// A page: its title (on a Mac; a phone's bar names it instead), then one grid, as the Home Screen's: the widgets its
/// person put there in their sizes and its apps' icons, in their order, then the Connectors folder, there even with
/// none. Touch and hold empty space (or choose Edit Home Page) to arrange it: everything jiggles, and apps and
/// widgets drag anywhere, come off with their − badge, and widgets resize from their corner.
struct PageView: View {
    @Environment(Store.self) private var store
    var page: Page
    var open: (Route) -> Void
    @State private var width: CGFloat = 0
    @State private var viewport: CGFloat = 0
    @State private var deleting: AppInfo?
    @State private var error: String?
    @State private var drag: Drag?
    @State private var resize: Resize?
    /// The widget whose options are showing, by its copy's id.
    @State private var options: String?
    @State private var removed = 0

    /// An item being dragged: the page's order so far, where it started, how far it's gone, and what it's over.
    struct Drag: Equatable {
        var id: String
        var order: [PageItem]
        var start: CGPoint
        var translation: CGSize
        var over: String?
    }

    /// A widget being resized from its corner: the size it started at, and the size it is now.
    struct Resize: Equatable {
        var id: String
        var start: String
        var size: String
    }

    var body: some View {
        let editing = store.editing
        let items = drag?.order ?? store.items(on: page)
        let grid = GridMetrics(width: width)
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                #if os(macOS)
                PageHeader(title: page.title, subtitle: page.subtitle)
                #endif
                if store.apps(on: page).isEmpty && store.widgets(on: page).isEmpty {
                    EmptyPage(page: page, open: open)
                }
                if width > 0 {
                    HomeGrid(grid: grid) {
                        ForEach(items) { item in
                            cell(item, grid: grid, editing: editing)
                                .layoutValue(key: GridSpan.self, value: span(item, grid: grid))
                        }
                        IconButton(title: String(localized: "Connectors")) { if !editing { open(.connectors(page.scope)) } } icon: {
                            FolderTile(apps: store.connectors(on: page), size: grid.icon)
                        }
                        .modifier(Jiggle(on: editing))
                    }
                    .coordinateSpace(.named(gridSpace))
                    .overlay(alignment: .topLeading) { lifted(grid: grid) }
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
            .frame(maxWidth: .infinity, minHeight: viewport, alignment: .top)
            .background { emptySpace }
        }
        .onScrollGeometryChange(for: CGFloat.self) { $0.containerSize.height } action: { viewport = $1 }
        .groupedBackground()
        .refreshable { try? await store.refreshState(); await store.refreshApps() }
        .deleteAppDialog($deleting, error: $error)
        .overlay { optionsCard(grid: grid) }
        .sensoryFeedback(.impact(weight: .light), trigger: drag?.id) { _, new in new != nil }
        .sensoryFeedback(.selection, trigger: drag?.over) { _, new in new != nil }
        .sensoryFeedback(.selection, trigger: resize?.size) { old, new in old != nil && new != nil }
        .sensoryFeedback(.impact(flexibility: .soft), trigger: removed)
        #if os(macOS)
        .toolbar { EditToolbar(page: page, store: store) }
        #endif
    }

    /// The grid's empty space: touch and hold it to arrange the page, as on the Home Screen, or right-click it on a
    /// Mac; while arranging, a tap there is Done.
    private var emptySpace: some View {
        Color.clear
            .contentShape(.rect)
            .onTapGesture { if store.editing { withAnimation { store.editing = false } } }
            #if os(iOS)
            .onLongPressGesture(minimumDuration: 0.45) { withAnimation { store.editing = true } }
            #else
            .contextMenu { Button("Edit Home Page", systemImage: "square.grid.3x3") { withAnimation { store.editing = true } } }
            #endif
    }

    private func size(_ widget: WidgetInfo, grid: GridMetrics) -> String {
        resize.flatMap { $0.id == widget.instance ? $0.size : nil } ?? grid.fit(widget.size ?? "medium")
    }

    private func span(_ item: PageItem, grid: GridMetrics) -> (w: Int, h: Int) {
        if case .widget(let widget, _) = item { return WidgetSize.cells(size(widget, grid: grid)) }
        return (1, 1)
    }

    /// One app's icon or widget on the grid, with its − badge, resize handle and jiggle while arranging.
    private func cell(_ item: PageItem, grid: GridMetrics, editing: Bool) -> some View {
        let lifted = drag?.id == item.id
        // The drag goes on the item itself, under its badge and handle, so they keep their own taps and drags.
        return content(item, grid: grid, editing: editing)
            .highPriorityGesture(move(item, grid: grid), isEnabled: editing)
            .overlay(alignment: .topLeading) {
                if editing {
                    RemoveBadge(label: removeLabel(item)) {
                        removed += 1
                        withAnimation(.snappy) { store.remove(item, from: page) }
                    }
                    .offset(x: item.isApp ? (grid.cell - grid.icon) / 2 - 9 : -9, y: -9)
                    .transition(.scale.combined(with: .opacity))
                }
            }
            .overlay(alignment: .bottomTrailing) {
                if editing, case .widget(let widget, let kept) = item, grid.sizes(of: widget).count > 1 {
                    ResizeHandle(grid: grid, sizes: grid.sizes(of: widget), current: size(widget, grid: grid)) { new in
                        if resize == nil { resize = Resize(id: widget.instance ?? "", start: size(widget, grid: grid), size: new) }
                        if resize?.size != new { withAnimation(.snappy) { resize?.size = new } }
                    } ended: {
                        if let resize, resize.size != resize.start { store.resize(kept, to: resize.size, on: page) }
                        resize = nil
                    }
                    .transition(.opacity)
                }
            }
            .modifier(Jiggle(on: editing && !lifted))
            .opacity(lifted ? 0 : 1)
    }

    @ViewBuilder
    private func content(_ item: PageItem, grid: GridMetrics, editing: Bool) -> some View {
        switch item {
        case .app(let app):
            IconButton(title: app.label, badge: store.appBadge(app.name, on: page) ?? 0) { if !editing { open(store.open(app, from: page)) } } icon: {
                AppTile(app: app, seed: app.name, size: grid.icon)
            }
            .modifier(ContextMenu(enabled: !editing) {
                Button("Edit Home Page", systemImage: "square.grid.3x3") { withAnimation { store.editing = true } }
                AppActions(app: app, page: page, open: open, deleting: $deleting, error: $error)
            })
        case .widget(let widget, let kept):
            PageWidget(widget: widget, kept: kept, page: page, size: size(widget, grid: grid), grid: grid, editing: editing, open: open,
                       editOptions: { options = widget.instance }, remove: { removed += 1; withAnimation(.snappy) { store.remove(item, from: page) } })
        }
    }

    private func removeLabel(_ item: PageItem) -> String {
        switch item {
        case .app(let app): String(localized: "Remove \(app.label) from this page")
        case .widget(let widget, _): String(localized: "Remove \(widget.title ?? widget.widget)")
        }
    }

    /// Drags an item while arranging: it follows the finger, and over another item takes that one's place, the rest
    /// moving aside; past the last, it goes at the end.
    private func move(_ item: PageItem, grid: GridMetrics) -> some Gesture {
        DragGesture(minimumDistance: 3, coordinateSpace: .named(gridSpace))
            .onChanged { value in
                let order = drag?.order ?? store.items(on: page)
                let frames = grid.frames(order.map { span($0, grid: grid) })
                guard let from = order.firstIndex(where: { $0.id == item.id }) else { return }
                var d = drag ?? Drag(id: item.id, order: order, start: frames[from].origin, translation: .zero)
                d.translation = value.translation
                let over = zip(order, frames).first { pair in pair.0.id != item.id && pair.1.contains(value.location) }?.0
                var to: Int?
                if let over, over.id != d.over {
                    to = order.firstIndex(of: over)
                } else if over == nil, value.location.y > (frames.map(\.maxY).max() ?? 0), from != order.count - 1 {
                    to = order.count - 1
                }
                d.over = over?.id
                if let to {
                    d.order.move(fromOffsets: [from], toOffset: to > from ? to + 1 : to)
                    withAnimation(.snappy) { drag = d }
                } else {
                    drag = d
                }
            }
            .onEnded { _ in
                if let drag, drag.order.map(\.id) != store.items(on: page).map(\.id) { store.arrange(drag.order, on: page) }
                withAnimation(.snappy) { drag = nil }
            }
    }

    /// The item being dragged, lifted above the grid under the finger.
    @ViewBuilder
    private func lifted(grid: GridMetrics) -> some View {
        if let drag, let item = drag.order.first(where: { $0.id == drag.id }) {
            let cells = span(item, grid: grid)
            content(item, grid: grid, editing: true)
                .frame(width: grid.span(cells.w), height: grid.span(cells.h))
                .scaleEffect(1.07)
                .shadow(color: .black.opacity(0.22), radius: 14, y: 8)
                .offset(x: drag.start.x + drag.translation.width, y: drag.start.y + drag.translation.height)
                .allowsHitTesting(false)
        }
    }

    /// A widget's options, flipped round to face its person, the rest of the page dimmed.
    @ViewBuilder
    private func optionsCard(grid: GridMetrics) -> some View {
        if let id = options, let (widget, kept) = store.items(on: page).lazy.compactMap({ item -> (WidgetInfo, Arrangement.Item)? in
            if case .widget(let w, let k) = item, k.id == id { (w, k) } else { nil }
        }).first {
            WidgetOptionsCard(widget: widget, kept: kept, page: page, size: size(widget, grid: grid), grid: grid) { options = nil }
        }
    }
}

/// The named space a page's grid lays out in, which dragging measures in.
private let gridSpace = "home-grid"

extension PageItem {
    var isApp: Bool { if case .app = self { true } else { false } }
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

    /// Where views of these spans go, in order, each in the first place it fits, row by row, so icons fill the
    /// cells beside a small widget; widgets start on an even column, as on the Home Screen. And how many rows that takes.
    func places(_ spans: [(w: Int, h: Int)]) -> (cells: [(x: Int, y: Int)], rows: Int) {
        var taken: [[Bool]] = []
        var cells: [(x: Int, y: Int)] = []
        for span in spans {
            let w = min(span.w, columns), h = span.h
            var y = 0
            search: while true {
                for x in stride(from: 0, through: columns - w, by: w > 1 ? 2 : 1) {
                    if (y..<y + h).allSatisfy({ row in row >= taken.count || (x..<x + w).allSatisfy { !taken[row][$0] } }) {
                        while taken.count < y + h { taken.append(Array(repeating: false, count: columns)) }
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

    /// The frames views of these spans take, in the grid's own space.
    func frames(_ spans: [(w: Int, h: Int)]) -> [CGRect] {
        zip(places(spans).cells, spans).map { cell, span in
            CGRect(x: CGFloat(cell.x) * (self.cell + gap), y: CGFloat(cell.y) * (self.cell + gap), width: self.span(min(span.w, columns)), height: self.span(span.h))
        }
    }
}

/// How many cells across and down a view in a `HomeGrid` takes: one, unless it says.
struct GridSpan: LayoutValueKey {
    static let defaultValue = (w: 1, h: 1)
}

/// Lays its views out in a grid's cells in order, as `GridMetrics.places` says.
struct HomeGrid: Layout {
    var grid: GridMetrics

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = grid.places(subviews.map { $0[GridSpan.self] }).rows
        return CGSize(width: grid.span(grid.columns), height: rows > 0 ? grid.span(rows) : 0)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let frames = grid.frames(subviews.map { $0[GridSpan.self] })
        for (view, frame) in zip(subviews, frames) {
            view.place(at: CGPoint(x: bounds.minX + frame.minX, y: bounds.minY + frame.minY), anchor: .topLeading, proposal: ProposedViewSize(frame.size))
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

/// A widget on a page: tapping it opens where it links, or its app. Touch and hold it (right-click on a Mac) and it
/// lifts, the rest blurred, over a menu as the Home Screen's: its sizes across the top, then Edit Widget, Edit Home
/// Page and Remove Widget.
private struct PageWidget: View {
    @Environment(Store.self) private var store
    @Environment(\.openURL) private var openURL
    var widget: WidgetInfo
    var kept: Arrangement.Item
    var page: Page
    var size: String
    var grid: GridMetrics
    var editing: Bool
    var open: (Route) -> Void
    var editOptions: () -> Void
    var remove: () -> Void

    var body: some View {
        let app = store.app(widget.app)
        WidgetCard(widget: widget, app: app, size: size)
            .environment(\.widgetHost, host)
            .allowsHitTesting(!editing)
            .contentShape(.rect(cornerRadius: 22, style: .continuous))
            .onTapGesture { if !editing { follow(widget.tree(size).link) } }
            .modifier(ContextMenu(enabled: !editing, menu: { menu }, preview: {
                WidgetCard(widget: widget, app: app, size: size)
                    .frame(width: grid.span(WidgetSize.cells(size).w), height: grid.span(WidgetSize.cells(size).h))
            }))
    }

    @ViewBuilder
    private var menu: some View {
        let sizes = grid.sizes(of: widget)
        if sizes.count > 1 {
            // Unlabelled, as the Home Screen's row of sizes is.
            Picker(selection: Binding(get: { size }, set: { new in withAnimation(.snappy) { store.resize(kept, to: new, on: page) } })) {
                ForEach(sizes, id: \.self) { Label(WidgetSize.title($0), systemImage: WidgetSize.symbol($0)).tag($0) }
            } label: {
                EmptyView()
            }
            .pickerStyle(.palette)
        }
        if !(widget.options ?? []).isEmpty {
            Button("Edit Widget", systemImage: "info.circle", action: editOptions)
        }
        Button("Edit Home Page", systemImage: "square.grid.3x3") { withAnimation { store.editing = true } }
        if let error = widget.error {
            Button("Copy What Went Wrong", systemImage: "doc.on.doc") { copy(error) }
        }
        Button("Remove Widget", systemImage: "minus.circle", role: .destructive, action: remove)
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

/// A context menu that's there only while `enabled`, as an arranging page has none; with `preview`, what lifts.
struct ContextMenu<Menu: View, Preview: View>: ViewModifier {
    var enabled: Bool
    @ViewBuilder var menu: Menu
    @ViewBuilder var preview: Preview

    func body(content: Content) -> some View {
        if !enabled {
            content
        } else if Preview.self == EmptyView.self {
            content.contextMenu { menu }
        } else {
            content.contextMenu { menu } preview: { preview }
        }
    }
}

extension ContextMenu where Preview == EmptyView {
    init(enabled: Bool, @ViewBuilder menu: () -> Menu) {
        self.init(enabled: enabled, menu: menu, preview: { EmptyView() })
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
                 ? "Apps you add to \(page.title) show up here."
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
                    row(.store, value: "")
                    row(.groups, value: "")
                    row(.devices, value: "")
                }
                SettingsSection {
                    row(.vault, value: "")
                    row(.access, value: "")
                    row(.appearance, value: (Appearance(rawValue: appearance) ?? .automatic).title)
                }
                if store.isAdmin {
                    SettingsSection {
                        row(.users, value: "")
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
    }

    private func row(_ item: Setting, value: String, badge: Bool = false) -> some View {
        SettingsRow(item: item, value: value, badge: badge) { open(.setting(item)) }
    }

    /// The app's version and build, and the node's, for telling which build a device runs.
    private var version: String {
        let info = Bundle.main.infoDictionary ?? [:]
        let app = "\(store.config?.name ?? appName), app \(info["BuildCommit"] as? String ?? "")"
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
        case .vault: .indigo
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
        self.init(icon: app?.icon, seed: app?.name ?? seed, color: app?.color, image: app?.image, imageKind: app?.imageKind, size: size)
    }
}
