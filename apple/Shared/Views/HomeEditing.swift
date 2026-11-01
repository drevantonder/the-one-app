import SwiftUI

// Arranging a page, as the Home Screen is arranged: jiggling, − badges, resize handles, the Edit menu and Done, the
// widget gallery, and a widget's options on its back.

/// Rocks a view while its page is being arranged, each a little out of step with the rest.
struct Jiggle: ViewModifier {
    var on: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var period = Double.random(in: 0.12...0.15)
    @State private var lean = Bool.random()

    func body(content: Content) -> some View {
        content.phaseAnimator(on && !reduceMotion ? [false, true] : [false]) { view, phase in
            view.rotationEffect(.degrees(on && !reduceMotion ? (phase != lean ? 1.3 : -1.3) : 0))
        } animation: { _ in .easeInOut(duration: period) }
    }
}

/// The − at an icon's or widget's corner while arranging: takes it off the page.
struct RemoveBadge: View {
    var label: String
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: "minus")
                .font(.system(size: 11, weight: .heavy))
                .foregroundStyle(.primary)
                .frame(width: 22, height: 22)
                .background(.regularMaterial, in: .circle)
                .overlay(Circle().strokeBorder(.separator, lineWidth: 0.5))
                .shadow(color: .black.opacity(0.15), radius: 2, y: 1)
                .padding(8)
                .contentShape(.circle)
        }
        .buttonStyle(.plain)
        .padding(-8)
        .accessibilityLabel(label)
    }
}

/// The curved handle at a widget's bottom corner while arranging: dragged, the widget takes the size nearest the
/// finger, of those it draws.
struct ResizeHandle: View {
    var grid: GridMetrics
    var sizes: [String]
    var current: String
    var changed: (String) -> Void
    var ended: () -> Void
    @State private var start: String?

    var body: some View {
        Path { path in
            path.addArc(center: CGPoint(x: 12, y: 12), radius: 22, startAngle: .degrees(0), endAngle: .degrees(90), clockwise: false)
        }
        .stroke(.white, style: StrokeStyle(lineWidth: 4.5, lineCap: .round))
        .shadow(color: .black.opacity(0.35), radius: 3)
        .frame(width: 34, height: 34)
        .padding(10)
        .contentShape(.rect)
        .padding(-10)
        .gesture(
            DragGesture(minimumDistance: 1, coordinateSpace: .named("home-grid"))
                .onChanged { value in
                    let from = start ?? current
                    start = from
                    let cells = WidgetSize.cells(from), step = grid.cell + grid.gap
                    let w = Double(cells.w) + value.translation.width / step, h = Double(cells.h) + value.translation.height / step
                    let distance = { (size: String) in
                        let c = WidgetSize.cells(size)
                        return pow(Double(c.w) - w, 2) + pow(Double(c.h) - h, 2)
                    }
                    if let nearest = sizes.min(by: { distance($0) < distance($1) }) { changed(nearest) }
                }
                .onEnded { _ in
                    start = nil
                    ended()
                }
        )
        .accessibilityLabel("Resize")
        .accessibilityAdjustableAction { direction in
            guard let i = sizes.firstIndex(of: current) else { return }
            let next = direction == .increment ? i + 1 : i - 1
            if sizes.indices.contains(next) { changed(sizes[next]); ended() }
        }
    }
}

/// While arranging, the Edit menu: Add Widget, and the apps taken off this page, to put back.
struct EditMenu: View {
    @Environment(Store.self) private var store
    var page: Page

    var body: some View {
        Menu {
            Button("Add Widget", systemImage: "plus.rectangle.on.rectangle") { store.sheet = .addWidget(page.scope) }
            let hidden = store.hiddenApps(on: page)
            if !hidden.isEmpty {
                Section("Taken Off This Page") {
                    ForEach(hidden) { app in
                        Button(app.label, systemImage: "plus.circle") { withAnimation(.snappy) { store.show(app, on: page) } }
                    }
                }
            }
        } label: {
            Text("Edit")
        }
    }
}

/// While arranging a phone's page: Edit at the top left, Done at the top right, as on the Home Screen.
struct EditBar: View {
    @Environment(Store.self) private var store
    var page: Page?

    var body: some View {
        HStack {
            if let page {
                EditMenu(page: page)
                    .buttonStyle(.glass)
            }
            Spacer()
            Button("Done") { withAnimation { store.editing = false } }
                .buttonStyle(.glassProminent)
                .fontWeight(.semibold)
        }
        .padding(.horizontal, 16)
        .transition(.move(edge: .top).combined(with: .opacity))
    }
}

#if os(macOS)
/// A Mac page's toolbar: Edit Home Page; while arranging, the Edit menu and Done.
struct EditToolbar: ToolbarContent {
    var page: Page
    var store: Store

    var body: some ToolbarContent {
        if store.editing {
            ToolbarItem { EditMenu(page: page) }
            ToolbarItem(placement: .confirmationAction) {
                Button("Done") { withAnimation { store.editing = false } }
                    .buttonStyle(.borderedProminent)
            }
        } else {
            ToolbarItem {
                Button("Edit Home Page", systemImage: "square.grid.3x3") { withAnimation { store.editing = true } }
                    .help("Arrange this page, and add widgets")
            }
        }
    }
}
#endif

/// The sizes a gallery offers: all a widget draws, but extra large on a phone, which has no room for it.
@MainActor
private func gallerySizes(_ widget: WidgetInfo) -> [String] {
    #if os(iOS)
    if UIDevice.current.userInterfaceIdiom == .phone { return widget.supported.filter { $0 != "xl" } }
    #endif
    return widget.supported
}

/// A widget drawn at a phone page's size, scaled to `width`.
private struct ScaledWidget: View {
    var widget: WidgetInfo
    var app: AppInfo?
    var size: String
    var width: CGFloat

    var body: some View {
        let grid = GridMetrics(width: 360)
        let cells = WidgetSize.cells(size)
        let w = grid.span(cells.w), h = grid.span(cells.h), scale = width / w
        WidgetCard(widget: widget, app: app, size: size)
            .frame(width: w, height: h)
            .scaleEffect(scale, anchor: .topLeading)
            .frame(width: w * scale, height: h * scale, alignment: .topLeading)
            .allowsHitTesting(false)
            .shadow(color: .black.opacity(0.08), radius: 8, y: 3)
    }
}

/// A widget and size the gallery shows, to add.
private struct GalleryPick: Hashable {
    var app: String
    var widget: String
    var size: String
}

/// Add Widget: a search field, big live previews of the page's widgets two to a row, then the page's apps that have
/// widgets. An app shows its widgets side by side, each in every size, to add one to the page.
struct AddWidgetSheet: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    var scope: String
    @State private var search = ""
    @State private var path: [GalleryPick] = []
    @State private var width: CGFloat = 360

    var body: some View {
        NavigationStack(path: $path) {
            Group {
                if let page = store.page(scope) {
                    gallery(page)
                        .navigationTitle("Add Widget · \(page.title)")
                        .navigationDestination(for: GalleryPick.self) { pick in WidgetPager(page: page, pick: pick) { dismiss() } }
                }
            }
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel", role: .cancel) { dismiss() } }
            }
        }
        #if os(macOS)
        .frame(width: 520, height: 680)
        #endif
    }

    private func gallery(_ page: Page) -> some View {
        let widgets = store.widgets(on: page).filter(matches)
        let apps = apps(widgets)
        return ScrollView {
            VStack(alignment: .leading, spacing: 28) {
                if widgets.isEmpty {
                    ContentUnavailableView(search.isEmpty ? "No Widgets" : "No Results",
                                           systemImage: "rectangle.on.rectangle.slash",
                                           description: Text(search.isEmpty ? "No app on \(page.title) has a widget." : "No widget matches “\(search)”."))
                        .padding(.top, 40)
                } else {
                    suggestions(widgets)
                    VStack(spacing: 0) {
                        ForEach(Array(apps.enumerated()), id: \.element.name) { i, app in
                            if i > 0 { Divider().padding(.leading, 62) }
                            Button {
                                if let first = widgets.first(where: { $0.app == app.name }) {
                                    path.append(GalleryPick(app: app.name, widget: first.widget, size: gallerySizes(first).first ?? "medium"))
                                }
                            } label: {
                                HStack(spacing: 14) {
                                    AppTile(app: app, seed: app.name, size: 34)
                                    Text(app.label).foregroundStyle(.primary)
                                    Spacer()
                                    Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
                                }
                                .padding(.horizontal, 14)
                                .frame(minHeight: 52)
                                .contentShape(.rect)
                            }
                            .buttonStyle(.plain)
                        }
                    }
                    .groupedContainer()
                }
            }
            .padding(20)
            .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 - 40 }
        }
        .groupedBackground()
        #if os(iOS)
        .searchable(text: $search, placement: .navigationBarDrawer(displayMode: .always), prompt: "Search Widgets")
        #else
        .searchable(text: $search, prompt: "Search Widgets")
        #endif
    }

    /// Each widget once, as big as the sheet allows: small ones two to a row, the rest across it.
    private func suggestions(_ widgets: [WidgetInfo]) -> some View {
        let picks = widgets.map { w in (widget: w, size: gallerySizes(w).first ?? w.size ?? "medium") }
        var rows: [[(widget: WidgetInfo, size: String)]] = []
        for pick in picks {
            if pick.size == "small", let last = rows.last, last.count == 1, last[0].size == "small" { rows[rows.count - 1].append(pick) } else { rows.append([pick]) }
        }
        let column = (width - 20) / 2
        return VStack(alignment: .leading, spacing: 22) {
            ForEach(Array(rows.enumerated()), id: \.offset) { _, row in
                HStack(alignment: .top, spacing: 20) {
                    ForEach(row, id: \.widget.id) { pick in
                        Button {
                            path.append(GalleryPick(app: pick.widget.app, widget: pick.widget.widget, size: pick.size))
                        } label: {
                            VStack(alignment: .leading, spacing: 8) {
                                ScaledWidget(widget: pick.widget, app: store.app(pick.widget.app), size: pick.size, width: pick.size == "small" ? column : width)
                                Text(store.app(pick.widget.app)?.label ?? pick.widget.app)
                                    .font(.footnote.weight(.medium))
                                    .foregroundStyle(.secondary)
                                    .lineLimit(1)
                            }
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
        }
    }

    private func matches(_ widget: WidgetInfo) -> Bool {
        let q = search.trimmingCharacters(in: .whitespaces)
        guard !q.isEmpty else { return true }
        return [widget.title, widget.description, store.app(widget.app)?.label].compactMap { $0 }.contains { $0.localizedCaseInsensitiveContains(q) }
    }

    /// The apps with widgets here, by name.
    private func apps(_ widgets: [WidgetInfo]) -> [AppInfo] {
        var seen = Set<String>()
        return widgets.compactMap { seen.insert($0.app).inserted ? store.app($0.app) : nil }
            .sorted { $0.label.localizedStandardCompare($1.label) == .orderedAscending }
    }
}

/// One app's widgets side by side, each kind in each size, with its name and a line on it, dots under them, and Add
/// Widget, which puts the one showing on the page.
private struct WidgetPager: View {
    @Environment(Store.self) private var store
    var page: Page
    var pick: GalleryPick
    var done: () -> Void
    @State private var position: GalleryPick?

    var body: some View {
        let picks = store.widgets(on: page).filter { $0.app == pick.app }.flatMap { w in gallerySizes(w).map { GalleryPick(app: w.app, widget: w.widget, size: $0) } }
        let current = position ?? pick
        VStack(spacing: 18) {
            ScrollView(.horizontal) {
                LazyHStack(spacing: 0) {
                    ForEach(picks, id: \.self) { p in
                        slide(p).containerRelativeFrame(.horizontal).id(p)
                    }
                }
                .scrollTargetLayout()
            }
            .scrollTargetBehavior(.paging)
            .scrollIndicators(.hidden)
            .scrollPosition(id: $position)
            if picks.count > 1 {
                HStack(spacing: 8) {
                    ForEach(picks, id: \.self) { p in
                        Circle().fill(p == current ? AnyShapeStyle(.primary) : AnyShapeStyle(.tertiary)).frame(width: 7, height: 7)
                            .onTapGesture { withAnimation { position = p } }
                    }
                }
                .accessibilityElement()
                .accessibilityLabel("Widget \((picks.firstIndex(of: current) ?? 0) + 1) of \(picks.count)")
            }
            Button {
                if let widget = store.widgets(on: page).first(where: { $0.app == current.app && $0.widget == current.widget }) {
                    withAnimation(.snappy) { store.add(widget, size: current.size, to: page) }
                }
                done()
            } label: {
                Label("Add Widget", systemImage: "plus").fontWeight(.semibold).frame(maxWidth: 260)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .padding(.bottom, 12)
        }
        .padding(.vertical, 12)
        .groupedBackground()
        .navigationTitle(store.app(pick.app)?.label ?? pick.app)
        .onAppear { if position == nil { position = pick } }
        .sensoryFeedback(.selection, trigger: position)
    }

    private func slide(_ p: GalleryPick) -> some View {
        let widget = store.widgets(on: page).first { $0.app == p.app && $0.widget == p.widget }
        return VStack(spacing: 6) {
            Text(widget?.title ?? p.widget).font(.title2.bold())
            Text(widget?.description.flatMap { $0.isEmpty ? nil : $0 } ?? WidgetSize.title(p.size))
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .lineLimit(2)
            GeometryReader { geo in
                if let widget {
                    let cells = WidgetSize.cells(p.size)
                    let grid = GridMetrics(width: 360)
                    let aspect = grid.span(cells.w) / grid.span(cells.h)
                    let natural = grid.span(cells.w)
                    let width = min(natural, geo.size.width - 40, (geo.size.height - 10) * aspect)
                    ScaledWidget(widget: widget, app: store.app(p.app), size: p.size, width: width)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            }
            .padding(.top, 16)
        }
        .padding(.horizontal, 20)
    }
}

/// A widget's options, iOS style: the widget flips round to show them, the rest dimmed; each change shows at once.
struct WidgetOptionsCard: View {
    @Environment(Store.self) private var store
    var widget: WidgetInfo
    var kept: Arrangement.Item
    var page: Page
    var size: String
    var grid: GridMetrics
    var close: () -> Void
    @State private var angle = 0.0
    @State private var back = false
    @State private var dim = false

    var body: some View {
        let cells = WidgetSize.cells(size)
        let front = CGSize(width: grid.span(cells.w), height: grid.span(cells.h))
        let card = CGSize(width: max(front.width, min(grid.span(4), 380)), height: max(front.height, 56 + CGFloat(widget.options?.count ?? 0) * 51 + 8))
        ZStack {
            Rectangle()
                .fill(.ultraThinMaterial)
                .overlay(Color.black.opacity(0.2))
                .ignoresSafeArea()
                .opacity(dim ? 1 : 0)
                .onTapGesture(perform: dismiss)
            Group {
                if back {
                    form.frame(width: card.width, height: card.height).rotation3DEffect(.degrees(180), axis: (x: 0, y: 1, z: 0))
                } else {
                    WidgetCard(widget: widget, app: store.app(widget.app), size: size).frame(width: front.width, height: front.height)
                }
            }
            .rotation3DEffect(.degrees(angle), axis: (x: 0, y: 1, z: 0), perspective: 0.4)
            .shadow(color: .black.opacity(0.2), radius: 20, y: 10)
        }
        .onAppear {
            withAnimation(.easeOut(duration: 0.2)) { dim = true }
            withAnimation(.easeIn(duration: 0.18)) { angle = 90 } completion: {
                back = true
                withAnimation(.spring(duration: 0.35, bounce: 0.25)) { angle = 180 }
            }
        }
        .sensoryFeedback(.impact(weight: .light), trigger: back)
    }

    private func dismiss() {
        withAnimation(.easeIn(duration: 0.16)) { angle = 90 } completion: {
            back = false
            withAnimation(.spring(duration: 0.3, bounce: 0.2)) { angle = 0 }
            withAnimation(.easeOut(duration: 0.3)) { dim = false } completion: { close() }
        }
    }

    /// The item as it's kept now, with every change made so far.
    private var current: Arrangement.Item {
        store.items(on: page).lazy.compactMap { item -> Arrangement.Item? in
            if case .widget(_, let k) = item, k.id == kept.id { k } else { nil }
        }.first ?? kept
    }

    private func value(_ option: WidgetOption) -> JSONValue {
        current.options?[option.name] ?? option.defaultValue ?? .null
    }

    private func set(_ option: WidgetOption, _ new: JSONValue) {
        store.setOption(option.name, new == (option.defaultValue ?? .null) ? nil : new, of: current, on: page)
    }

    private var form: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 10) {
                AppTile(app: store.app(widget.app), seed: widget.app, size: 28)
                Text(widget.title ?? widget.widget).font(.headline)
                Spacer()
            }
            .padding(.horizontal, 16)
            .frame(height: 56)
            ForEach(widget.options ?? []) { option in
                Divider().padding(.leading, 16)
                HStack {
                    Text(option.title)
                    Spacer(minLength: 12)
                    if option.isSwitch {
                        Toggle(option.title, isOn: Binding(get: { value(option) == .bool(true) }, set: { set(option, .bool($0)) }))
                            .labelsHidden()
                    } else if let choices = option.choices {
                        Picker(option.title, selection: Binding(get: { value(option) }, set: { set(option, $0) })) {
                            ForEach(choices, id: \.self) { Text($0.title).tag($0.value) }
                        }
                        .labelsHidden()
                        .pickerStyle(.menu)
                    }
                }
                .padding(.horizontal, 16)
                .frame(minHeight: 50)
            }
            Spacer(minLength: 0)
        }
        .background(cardBackground, in: .rect(cornerRadius: 22, style: .continuous))
    }

    private var cardBackground: some ShapeStyle {
        #if os(macOS)
        AnyShapeStyle(.background)
        #else
        AnyShapeStyle(Color(.secondarySystemGroupedBackground))
        #endif
    }
}
