import Charts
import SwiftUI

// Draws a widget with SwiftUI's own parts, so it follows the system's light,
// dark and tinted looks. Ported from the PTC App's home-screen widget.

/// Runs a button's or toggle's action; nil where widgets can't act.
typealias WidgetRun = (WidgetAction) async throws -> Void

struct WidgetContext {
    var run: WidgetRun?
    var images: [String: Image] = [:]
    /// On a home screen, actions run as app intents instead.
    var homeScreen = false
    /// The widget's plugin, whose operations its actions name.
    var app = ""
    /// Opens a live web page in a window of its own; nil where that can't happen.
    var popout: (@MainActor (URL, String) -> Void)? = WidgetContext.popouts
    /// How the app opens popouts; home-screen widgets have none.
    nonisolated(unsafe) static var popouts: (@MainActor (URL, String) -> Void)?

    func tool(_ op: String) -> String { app == "one" ? op : "\(app)_\(op)" }
}

private func sizeOf(_ size: String?) -> CGFloat {
    switch size { case "xl": 28; case "lg": 20; case "sm": 13; case "xs": 10; default: 15 }
}

private func weightOf(_ weight: String?) -> Font.Weight {
    switch weight { case "bold": .semibold; case "medium": .medium; default: .regular }
}

func widgetColor(_ name: String?) -> Color {
    switch name {
    case "green": .green
    case "orange": .orange
    case "red": .red
    case "blue": Color(red: 0x0a / 255, green: 0x9e / 255, blue: 0xdc / 255)
    case "purple": Color(red: 0xa5 / 255, green: 0x50 / 255, blue: 0xd9 / 255)
    case "gray": .gray
    case "muted": .secondary
    default: .primary
    }
}

extension View {
    func styled(_ size: String?, _ weight: String?, _ tone: String?) -> some View {
        font(.system(size: sizeOf(size), weight: weightOf(weight)))
            .foregroundStyle(tone == "muted" ? AnyShapeStyle(.secondary) : AnyShapeStyle(.primary))
    }
}

struct WidgetView: View {
    let description: Description
    var ctx = WidgetContext()

    var body: some View {
        var ctx = ctx
        ctx.app = description.app
        return NodeView(node: description.root, axis: .vertical, ctx: ctx)
    }
}

struct NodeView: View {
    let node: Node
    let axis: Axis
    let ctx: WidgetContext

    var body: some View {
        switch node {
        case .stack(let s): StackView(stack: s, ctx: ctx)
        case .link(let l):
            if let url = URL(string: l.to) {
                Link(destination: url) { StackView(stack: Stack.column(l.children), ctx: ctx) }.buttonStyle(.plain)
            } else {
                StackView(stack: Stack.column(l.children), ctx: ctx)
            }
        case .text(let t):
            let view = Text(t.text).styled(t.size, t.weight, t.tone).lineLimit(t.lines)
                .multilineTextAlignment(t.align == "center" ? .center : t.align == "end" ? .trailing : .leading)
            switch t.anim {
            case "numeric": view.contentTransition(.numericText())
            case "fade": view.contentTransition(.opacity)
            default: view
            }
        case .time(let t): TimeView(time: t)
        case .image(let i): ImageView(node: i, image: ctx.images[i.src])
        case .spacer(let s):
            if let size = s.size { Spacer().frame(width: axis == .horizontal ? size : nil, height: axis == .vertical ? size : nil) } else { Spacer(minLength: 0) }
        case .stat(let s):
            VStack(alignment: .leading, spacing: 0) {
                Text(s.value).font(.system(size: 28, weight: .bold)).contentTransition(s.anim == "fade" ? .opacity : .numericText())
                if let label = s.label { Text(label).font(.system(size: 13)).foregroundStyle(.secondary) }
            }
        case .list(let l): ListView(items: l.items, grow: l.grow == true, ctx: ctx)
        case .chart(let c): ChartView(chart: c)
        case .button(let b): ActionButton(node: b, ctx: ctx)
        case .toggle(let t): ActionToggle(node: t, ctx: ctx)
        case .progress(let p): ProgressBar(node: p)
        case .gauge(let g):
            let size = g.size ?? 18
            ZStack {
                Circle().stroke(Color.secondary.opacity(0.25), lineWidth: size / 6)
                Circle().trim(from: 0, to: min(1, max(0, g.value)))
                    .stroke(widgetColor(g.color), style: StrokeStyle(lineWidth: size / 6, lineCap: .round))
                    .rotationEffect(.degrees(-90))
            }
            .padding(size / 12)
            .frame(width: size, height: size)
        case .spark(let s): SparkView(node: s)
        case .icon(let i): IconView(node: i)
        case .dot(let d): Circle().fill(widgetColor(d.color ?? "green")).frame(width: 7, height: 7)
        case .grid(let g):
            LazyVGrid(columns: [GridItem(.adaptive(minimum: g.min), spacing: g.gap ?? 12, alignment: .top)], alignment: .leading, spacing: g.gap ?? 12) {
                ForEach(Array(g.children.enumerated()), id: \.offset) { _, child in NodeView(node: child, axis: .vertical, ctx: ctx) }
            }
        case .popout(let p):
            let content = StackView(stack: Stack.column(p.children), ctx: ctx)
            if let url = URL(string: p.url), let popout = ctx.popout {
                Button { popout(url, p.title) } label: { content.contentShape(Rectangle()) }.buttonStyle(.plain)
            } else if let url = URL(string: p.url) {
                Link(destination: url) { content }.buttonStyle(.plain)
            } else {
                content
            }
        case .unknown: EmptyView()
        }
    }
}

struct StackView: View {
    let stack: Stack
    let ctx: WidgetContext

    var body: some View {
        let axis: Axis = stack.vertical ? .vertical : .horizontal
        let gap = stack.gap ?? (stack.vertical ? 4 : 8)
        Group {
            if stack.vertical {
                VStack(alignment: horizontalAlign, spacing: gap) { content(axis) }
            } else {
                HStack(alignment: verticalAlign, spacing: gap) { content(axis) }
            }
        }
        .padding(stack.pad ?? 0)
        .frame(width: stack.w)
        .modifier(Box(aspect: stack.aspect, radius: stack.radius ?? 0, border: stack.border))
        .opacity(stack.dim == true ? 0.5 : 1)
    }

    private var horizontalAlign: HorizontalAlignment {
        switch stack.align { case "center": .center; case "end": .trailing; default: .leading }
    }

    private var verticalAlign: VerticalAlignment {
        switch stack.align { case "start": .top; case "end": .bottom; default: .center }
    }

    @ViewBuilder private func content(_ axis: Axis) -> some View {
        let children = stack.children
        let spread = stack.justify
        if spread == "center" || spread == "end" { Spacer(minLength: 0) }
        ForEach(Array(children.enumerated()), id: \.offset) { index, child in
            if spread == "between", index > 0 { Spacer(minLength: 0) }
            NodeView(node: child, axis: axis, ctx: ctx)
                .frame(maxWidth: fillsWidth(axis, child) ? .infinity : nil, maxHeight: axis == .vertical && stack.grow(child) ? .infinity : nil,
                       alignment: stack.align == "center" ? .center : .leading)
        }
        if spread == "center" { Spacer(minLength: 0) }
    }

    /// Columns stretch their children across by default; any node with `grow` takes the free space along its parent's axis.
    private func fillsWidth(_ axis: Axis, _ child: Node) -> Bool {
        axis == .horizontal ? stack.grow(child) : (stack.align ?? "stretch") == "stretch"
    }
}

/// Gives a stack a fixed shape and an outline.
struct Box: ViewModifier {
    let aspect: CGFloat?
    let radius: CGFloat
    let border: String?

    func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: radius, style: .continuous)
        let sized = Group {
            if let aspect {
                Color.clear.aspectRatio(aspect, contentMode: .fit).overlay { content }
            } else {
                content
            }
        }
        switch border {
        case "dashed": sized.clipShape(shape).overlay { shape.strokeBorder(Color.secondary.opacity(0.5), style: StrokeStyle(lineWidth: 1.5, dash: [5, 4])) }
        case "solid": sized.clipShape(shape).overlay { shape.strokeBorder(Color.secondary.opacity(0.35), lineWidth: 1) }
        default: sized.clipShape(shape)
        }
    }
}

extension Stack {
    func grow(_ node: Node) -> Bool {
        switch node {
        case .stack(let s): return s.grow == true
        case .list(let l): return l.grow == true
        case .chart(let c): return c.grow == true
        default: return false
        }
    }
}

struct TimeView: View {
    let time: TimeNode

    var body: some View {
        let date = Date(timeIntervalSince1970: time.at / 1000)
        Group {
            switch time.style {
            case "timer": Text(date, style: .timer).monospacedDigit()
            case "date": Text(date, style: .date)
            case "clock": Text(date, style: .time)
            default: Text(relative(date))
            }
        }
        .styled(time.size, time.weight, time.tone).lineLimit(1)
    }

    /// Worded as every renderer words it; the widget is drawn again at `next`.
    private func relative(_ date: Date) -> String {
        let minutes = Int((Date().timeIntervalSince(date) / 60).rounded())
        let size = abs(minutes)
        let span = size < 1 ? "now" : size < 60 ? "\(size) min" : size < 1440 ? "\(Int((Double(size) / 60).rounded())) hr" : "\(Int((Double(size) / 1440).rounded())) d"
        return size < 1 ? span : minutes > 0 ? "\(span) ago" : "in \(span)"
    }
}

struct ImageView: View {
    let node: ImageNode
    let image: Image?

    var body: some View {
        let picture = Group {
            if let image {
                image.resizable().scaledToFill()
            } else if let data = dataURL(node.src), let image = platformImage(data) {
                image.resizable().scaledToFill()
            } else if let url = URL(string: node.src) {
                AsyncImage(url: url) { $0.resizable().scaledToFill() } placeholder: { Color.secondary.opacity(0.15) }
            } else {
                Color.secondary.opacity(0.15)
            }
        }
        let shape = RoundedRectangle(cornerRadius: node.round == true ? 1000 : node.radius ?? 0, style: .continuous)
        Group {
            if let aspect = node.aspect {
                Color.clear.aspectRatio(aspect, contentMode: .fit).overlay { picture }
            } else {
                picture.frame(width: node.w ?? 28, height: node.h ?? 28)
            }
        }
        .clipShape(shape)
        .accessibilityLabel(node.alt ?? "")
    }
}

/// The bytes of a base64 data: URL.
func dataURL(_ src: String) -> Data? {
    guard src.hasPrefix("data:"), let comma = src.firstIndex(of: ",") else { return nil }
    return Data(base64Encoded: String(src[src.index(after: comma)...]))
}

func platformImage(_ data: Data) -> Image? {
    #if os(macOS)
    NSImage(data: data).map(Image.init(nsImage:))
    #else
    UIImage(data: data).map(Image.init(uiImage:))
    #endif
}

/// Shows as many items as fit, then "+N more"; when some are hidden the shown rows spread to fill the height.
struct ListView: View {
    let items: [Item]
    let grow: Bool
    let ctx: WidgetContext

    var body: some View {
        let tiles = items.contains { $0.img != nil }
        // More than a dozen rows never fit a widget; trying every count of a long list would build hundreds of rows.
        ViewThatFits(in: .vertical) {
            ForEach(Array(stride(from: min(items.count, 12), through: 1, by: -1)), id: \.self) { count in
                let hidden = count < items.count
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(items.prefix(count).enumerated()), id: \.offset) { index, item in
                        if index > 0 { if hidden { Spacer(minLength: 10) } else { Spacer().frame(height: 10) } }
                        ItemView(item: item, tile: tiles, ctx: ctx)
                    }
                    if hidden {
                        Spacer(minLength: 10)
                        Text("+\(items.count - count) more").font(.system(size: 13)).foregroundStyle(.secondary)
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: hidden ? .infinity : nil, alignment: .topLeading)
            }
        }
        .frame(maxHeight: grow ? .infinity : nil, alignment: .top)
    }
}

struct ItemView: View {
    let item: Item
    let tile: Bool
    let ctx: WidgetContext

    var body: some View {
        if let url = item.link.flatMap(URL.init(string:)) {
            Link(destination: url) { row }.buttonStyle(.plain)
        } else {
            row
        }
    }

    private var row: some View {
        HStack(spacing: 10) {
            if let img = item.img {
                ImageView(node: ImageNode(src: img.src, w: img.w ?? 28, h: img.h ?? 28, round: img.round, radius: img.round == true ? nil : img.radius ?? 7, alt: img.alt),
                          image: ctx.images[img.src])
            } else if tile {
                RoundedRectangle(cornerRadius: 7, style: .continuous).fill(Color.secondary.opacity(0.12)).frame(width: 28, height: 28)
            }
            VStack(alignment: .leading, spacing: 1) {
                Text(item.title).font(.system(size: 15, weight: .medium)).lineLimit(1)
                if let sub = item.sub { Text(sub).font(.system(size: 13)).foregroundStyle(.secondary).lineLimit(1) }
            }
            Spacer(minLength: 0)
            if let end = item.end?.value { NodeView(node: end, axis: .horizontal, ctx: ctx) }
        }
    }
}

struct ChartView: View {
    let chart: ChartNode

    var body: some View {
        Chart(Array(chart.data.enumerated()), id: \.offset) { index, point in
            let highlighted = index == chart.hl
            if chart.kind == "bar" {
                BarMark(x: .value("Index", index), y: .value("Value", point.value))
                    .foregroundStyle(highlighted ? AnyShapeStyle(.primary) : AnyShapeStyle(Color.secondary.opacity(0.35)))
                    .annotation(position: .top) { if highlighted { label(point) } }
            } else {
                LineMark(x: .value("Index", index), y: .value("Value", point.value)).foregroundStyle(Color.secondary.opacity(0.5))
                if highlighted {
                    PointMark(x: .value("Index", index), y: .value("Value", point.value)).foregroundStyle(.primary)
                        .annotation(position: .top) { label(point) }
                }
            }
        }
        .chartXAxis(.hidden)
        .chartYAxis(.hidden)
        .frame(minHeight: chart.h ?? 60, maxHeight: chart.grow == true ? .infinity : chart.h ?? 60)
    }

    private func label(_ point: ChartPoint) -> some View {
        Text(point.label ?? point.value.formatted(.number.precision(.fractionLength(0...1)))).font(.system(size: 11, weight: .semibold))
    }
}

struct ProgressBar: View {
    let node: ProgressNode

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            if let label = node.label { Text(label).font(.system(size: 13)).foregroundStyle(.secondary) }
            GeometryReader { geo in
                ZStack(alignment: .leading) {
                    Capsule().fill(Color.secondary.opacity(0.25))
                    Capsule().fill(widgetColor(node.color))
                        .frame(width: max(6, geo.size.width * min(1, max(0, node.value))))
                }
            }
            .frame(width: node.w, height: 6)
        }
    }
}

struct SparkView: View {
    let node: SparkNode

    var body: some View {
        let color = widgetColor(node.color)
        Canvas { context, size in
            guard !node.data.isEmpty else {
                var line = Path()
                line.move(to: CGPoint(x: 4, y: size.height / 2))
                line.addLine(to: CGPoint(x: size.width - 4, y: size.height / 2))
                context.stroke(line, with: .color(.secondary.opacity(0.3)), style: StrokeStyle(lineWidth: 1.5, dash: [2, 3]))
                return
            }
            let data = node.data.count == 1 ? [node.data[0], node.data[0]] : node.data
            let top = node.max ?? max(1, data.max() ?? 1)
            let points = data.enumerated().map { i, v in
                CGPoint(x: CGFloat(i) / CGFloat(data.count - 1) * size.width,
                        y: size.height - 1 - CGFloat(min(v, top) / top) * (size.height - 2))
            }
            var line = Path()
            line.addLines(points)
            var area = line
            area.addLine(to: CGPoint(x: size.width, y: size.height))
            area.addLine(to: CGPoint(x: 0, y: size.height))
            area.closeSubpath()
            context.fill(area, with: .color(color.opacity(0.2)))
            context.stroke(line, with: .color(color), style: StrokeStyle(lineWidth: 1.4, lineJoin: .round))
        }
        .frame(width: node.w, height: node.h)
    }
}

/// An SVG icon, with an optional badge at its bottom trailing corner and the
/// icon cut away around it.
struct IconView: View {
    let node: IconNode

    var body: some View {
        let size = node.size ?? 22
        let shape = SVGShape(node.path)
        ZStack {
            Group {
                if node.stroke == true {
                    shape.stroke(style: StrokeStyle(lineWidth: 1.5 * size / 24, lineJoin: .round))
                } else {
                    shape.fill()
                }
            }
            .mask {
                if node.badge != nil {
                    Rectangle().overlay(Circle().frame(width: size * 14 / 24, height: size * 14 / 24).offset(x: size * 9 / 24, y: size * 9 / 24).blendMode(.destinationOut))
                        .compositingGroup()
                } else {
                    Rectangle()
                }
            }
            if let badge = node.badge {
                SVGShape(badge).fill()
                    .frame(width: size * 11 / 24, height: size * 11 / 24)
                    .offset(x: size * 9 / 24, y: size * 9 / 24)
            }
        }
        .frame(width: size, height: size)
    }
}

struct ActionButton: View {
    let node: ButtonNode
    let ctx: WidgetContext
    @State private var busy = false

    var body: some View {
        if ctx.homeScreen {
            Button(intent: RunWidgetAction(tool: ctx.tool(node.action.op), input: node.action.input)) { label }.buttonStyle(.plain)
        } else {
            Button {
                Task {
                    busy = true
                    try? await ctx.run?(node.action)
                    busy = false
                }
            } label: { label }
            .buttonStyle(.plain)
            .disabled(ctx.run == nil || busy)
        }
    }

    private var label: some View {
        Text(node.label).font(.system(size: 13, weight: .semibold)).lineLimit(1)
            .padding(.horizontal, 12).padding(.vertical, 6)
            .foregroundStyle(node.primary == true ? AnyShapeStyle(.white) : AnyShapeStyle(.primary))
            .background(node.primary == true ? AnyShapeStyle(Color.accentColor) : AnyShapeStyle(Color.secondary.opacity(0.2)), in: Capsule())
            .opacity(busy ? 0.5 : 1)
    }
}

struct ActionToggle: View {
    let node: ToggleNode
    let ctx: WidgetContext
    @State private var on: Bool?

    var body: some View {
        if ctx.homeScreen {
            Toggle(isOn: node.on, intent: RunWidgetAction(tool: ctx.tool(node.action.op), input: node.action.input)) { label }
        } else {
            Toggle(isOn: Binding(get: { on ?? node.on }, set: { value in
                on = value
                Task {
                    do { try await ctx.run?(node.action) } catch { on = nil }
                }
            })) { label }
            .disabled(ctx.run == nil)
            .onChange(of: node.on) { on = nil }
        }
    }

    private var label: some View { Text(node.label).font(.system(size: 15)) }
}
