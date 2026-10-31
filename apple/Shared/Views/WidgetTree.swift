import Charts
import SwiftUI

/// A rendered widget, as `GET /v1/widgets` gives it.
struct WidgetInfo: Codable, Sendable, Equatable, Identifiable {
    var app: String
    var widget: String
    var title: String?
    /// The size a page starts it at: `small`, `medium`, `large` or `xl`.
    var size: String?
    /// Its tree at each size it draws; `root` is the one at `size`.
    var sizes: [String: WidgetNode]?
    /// Why its render failed, if it did; `root` then says so for shells that don't know this.
    var error: String?
    var root: WidgetNode

    var id: String { "\(app).\(widget)" }

    /// The sizes it draws, smallest first: every size, from a server that renders each widget once.
    var supported: [String] { WidgetSize.all.filter { sizes?[$0] != nil || sizes == nil } }

    /// Its tree at `size`, or at its own size if it doesn't draw that one.
    func tree(_ size: String) -> WidgetNode { sizes?[size] ?? root }
}

/// The sizes of an iPhone's Home Screen widgets, in its grid's cells: small 2×2, medium 4×2, large 4×4, and xl 8×4,
/// where a page is wide enough.
enum WidgetSize {
    static let all = ["small", "medium", "large", "xl"]

    static func cells(_ size: String) -> (w: Int, h: Int) {
        switch size {
        case "small": (2, 2)
        case "large": (4, 4)
        case "xl": (8, 4)
        default: (4, 2)
        }
    }

    static func title(_ size: String) -> String {
        switch size {
        case "small": String(localized: "Small")
        case "large": String(localized: "Large")
        case "xl": String(localized: "Extra Large")
        default: String(localized: "Medium")
        }
    }

    static func symbol(_ size: String) -> String {
        switch size {
        case "small": "square"
        case "large": "square.grid.2x2"
        case "xl": "rectangle.split.2x1"
        default: "rectangle"
        }
    }
}

/// Any JSON value: an operation's input, as a widget gives it.
enum JSONValue: Codable, Sendable, Hashable {
    case null, bool(Bool), number(Double), string(String), array([JSONValue]), object([String: JSONValue])

    init(from decoder: Decoder) throws {
        let value = try decoder.singleValueContainer()
        if value.decodeNil() { self = .null }
        else if let v = try? value.decode(Bool.self) { self = .bool(v) }
        else if let v = try? value.decode(Double.self) { self = .number(v) }
        else if let v = try? value.decode(String.self) { self = .string(v) }
        else if let v = try? value.decode([JSONValue].self) { self = .array(v) }
        else { self = .object(try value.decode([String: JSONValue].self)) }
    }

    func encode(to encoder: Encoder) throws {
        var value = encoder.singleValueContainer()
        switch self {
        case .null: try value.encodeNil()
        case .bool(let v): try value.encode(v)
        case .number(let v): try value.encode(v)
        case .string(let v): try value.encode(v)
        case .array(let v): try value.encode(v)
        case .object(let v): try value.encode(v)
        }
    }

    var data: Data { (try? JSONEncoder().encode(self)) ?? Data("null".utf8) }

    /// Its text, as a stat shows it: numbers without a needless ".0".
    var text: String {
        switch self {
        case .number(let n): n.formatted(.number.precision(.fractionLength(0...2)))
        case .string(let s): s
        case .bool(let b): b ? "Yes" : "No"
        default: ""
        }
    }

    var number: Double? {
        switch self {
        case .number(let n): n
        case .string(let s): Double(s)
        default: nil
        }
    }

    func merging(_ key: String, _ value: JSONValue) -> JSONValue {
        guard case .object(var fields) = self else { return .object([key: value]) }
        fields[key] = value
        return .object(fields)
    }
}

/// Where tapping a node goes: a web page, or one of its app's screens.
enum WidgetLink: Codable, Sendable, Hashable {
    case url(URL)
    case screen(String, params: JSONValue?)

    private struct Screen: Codable { var screen: String; var params: JSONValue? }

    init(from decoder: Decoder) throws {
        let value = try decoder.singleValueContainer()
        if let text = try? value.decode(String.self) {
            guard let url = URL(string: text), url.scheme == "https" || url.scheme == "http" else {
                throw DecodingError.dataCorruptedError(in: value, debugDescription: "not a web address")
            }
            self = .url(url)
        } else {
            let screen = try value.decode(Screen.self)
            self = .screen(screen.screen, params: screen.params)
        }
    }

    func encode(to encoder: Encoder) throws {
        var value = encoder.singleValueContainer()
        switch self {
        case .url(let url): try value.encode(url.absoluteString)
        case .screen(let screen, let params): try value.encode(Screen(screen: screen, params: params))
        }
    }

    /// The link as a URL the app opens: web pages as they are, screens as `<cli>://open?…`.
    func url(app: String, scope: String?) -> URL? {
        switch self {
        case .url(let url): return url
        case .screen(let screen, let params):
            let cli = Bundle.main.object(forInfoDictionaryKey: "BrandCLI") as? String ?? "one"
            var parts = URLComponents()
            parts.scheme = cli
            parts.host = "open"
            parts.queryItems = [
                URLQueryItem(name: "app", value: app),
                URLQueryItem(name: "screen", value: screen),
                URLQueryItem(name: "params", value: String(decoding: (params ?? .object([:])).data, as: UTF8.self)),
            ] + (scope.map { [URLQueryItem(name: "scope", value: $0)] } ?? [])
            return parts.url
        }
    }
}

/// A node of a widget's tree (SPEC.md "Widgets"). Fields a renderer doesn't know are skipped, and so are children
/// it can't read.
struct WidgetNode: Codable, Sendable, Equatable {
    var t: String
    var gap: Double?
    var c: [WidgetNode]?
    var text: String?
    var size: String?
    var weight: String?
    var tone: String?
    var lines: Int?
    var at: Double?
    var style: String?
    var value: JSONValue?
    var max: Double?
    var label: String?
    var unit: String?
    var values: [Double]?
    var labels: [String]?
    var kind: String?
    var title: String?
    var subtitle: String?
    var trailing: String?
    var icon: String?
    var name: String?
    var url: String?
    var w: Double?
    var h: Double?
    var round: Bool?
    var empty: String?
    var link: WidgetLink?
    var op: String?
    var input: JSONValue?
    var on: Bool?
    var anim: Bool?
    var next: Double?
    var grow: Bool?
    var align: String?
    var dim: Bool?
    var badge: String?
    /// On the root, false: the tree labels itself, so no title goes over it.
    var header: Bool?

    init(t: String, gap: Double? = nil, c: [WidgetNode]? = nil, text: String? = nil, size: String? = nil, weight: String? = nil,
         tone: String? = nil, lines: Int? = nil, at: Double? = nil, value: JSONValue? = nil, label: String? = nil) {
        self.t = t
        self.gap = gap
        self.c = c
        self.text = text
        self.size = size
        self.weight = weight
        self.tone = tone
        self.lines = lines
        self.at = at
        self.value = value
        self.label = label
    }

    private enum Key: String, CodingKey {
        case t, gap, c, text, size, weight, tone, lines, at, style, value, max, label, unit, values, labels, kind, title, subtitle,
             trailing, icon, name, url, w, h, round, empty, link, op, input, on, anim, next, grow, align, dim, badge, header
    }

    /// Decodes what it can: a field of the wrong shape is left out, and so is a child that isn't a node.
    init(from decoder: Decoder) throws {
        let k = try decoder.container(keyedBy: Key.self)
        func get<T: Decodable>(_ key: Key) -> T? { (try? k.decodeIfPresent(T.self, forKey: key)) ?? nil }
        t = try k.decode(String.self, forKey: .t)
        gap = get(.gap); text = get(.text); size = get(.size); weight = get(.weight); tone = get(.tone); lines = get(.lines)
        at = get(.at); style = get(.style); value = get(.value); max = get(.max); label = get(.label); unit = get(.unit)
        values = get(.values); labels = get(.labels); kind = get(.kind); title = get(.title); subtitle = get(.subtitle)
        trailing = get(.trailing); icon = get(.icon); name = get(.name); url = get(.url); w = get(.w); h = get(.h)
        round = get(.round); empty = get(.empty); link = get(.link); op = get(.op); input = get(.input); on = get(.on)
        anim = get(.anim); next = get(.next); grow = get(.grow); align = get(.align); dim = get(.dim); badge = get(.badge)
        header = get(.header)
        let children: [Skippable]? = get(.c)
        c = children?.compactMap(\.node)
    }

    private struct Skippable: Decodable {
        var node: WidgetNode?
        init(from decoder: Decoder) throws { node = try? WidgetNode(from: decoder) }
    }

    /// Its most prominent text (the biggest, then the first that isn't muted): what a one-line place shows.
    var headline: String? {
        func texts(_ n: WidgetNode) -> [(String, Int, Bool)] {
            switch n.t {
            case "text": [(n.text ?? "", ["sm": 0, "md": 1, "lg": 2, "xl": 3][n.size ?? "md"] ?? 1, n.tone == "muted")]
            case "stat": [([n.value?.text ?? "", n.unit ?? ""].filter { !$0.isEmpty }.joined(separator: " ") + (n.label.map { " \($0)" } ?? ""), 4, false)]
            case "item": [(n.title ?? "", 1, false)]
            default: (n.c ?? []).flatMap(texts)
            }
        }
        return texts(self).filter { !$0.0.isEmpty }.enumerated().max { a, b in
            (a.element.1, a.element.2 ? 0 : 1, -a.offset) < (b.element.1, b.element.2 ? 0 : 1, -b.offset)
        }?.element.0
    }

    /// Every image the tree shows: a home-screen widget can only draw ones the app saved for it.
    var imageURLs: [URL] {
        (t == "image" ? [url.flatMap(URL.init(string:))].compactMap { $0 } : []) + (c ?? []).flatMap(\.imageURLs)
    }

    /// When its app wants it rendered again: `next`, kept between a minute and fifteen from now.
    func redraw(after now: Date = .now) -> Date? {
        next.map { Date(timeIntervalSince1970: $0 / 1000) }.map { Swift.min(Swift.max($0, now.addingTimeInterval(60)), now.addingTimeInterval(900)) }
    }
}

/// What a widget's buttons, toggles and links act through, where it's drawn. Left empty, they only show.
struct WidgetHost {
    var app = ""
    /// The page it's on, for the screens it opens.
    var scope: String?
    /// Runs an operation of the app; the error's text if it failed.
    var run: (@MainActor (_ op: String, _ input: JSONValue) async -> String?)?
    var open: (@MainActor (WidgetLink) -> Void)?
}

extension EnvironmentValues {
    @Entry var widgetHost = WidgetHost()
}

/// Draws a widget's tree. Text of the default size takes the surrounding font, so each place sets its own.
struct WidgetTree: View {
    var node: WidgetNode
    /// What `time` nodes count from: a home-screen widget passes its timeline entry's date.
    var now: Date = .now
    var inRow = false
    @Environment(\.widgetHost) private var host

    var body: some View {
        if let link = node.link, node.t != "item", node.t != "link" {
            Linked(link: link) { content }
        } else {
            content
        }
    }

    @ViewBuilder private var content: some View {
        switch node.t {
        case "col":
            VStack(alignment: node.align == "center" ? .center : .leading, spacing: node.gap.map { CGFloat($0) } ?? 4) { children(inRow: false) }
                .frame(width: node.w.map { CGFloat($0) })
                .opacity(node.dim == true ? 0.5 : 1)
        case "row":
            HStack(alignment: .center, spacing: node.gap.map { CGFloat($0) } ?? 6) { children(inRow: true) }
                .frame(width: node.w.map { CGFloat($0) }, alignment: node.align == "center" ? .center : .leading)
                .opacity(node.dim == true ? 0.5 : 1)
        case "text":
            Changing(text: node.text ?? "", number: nil, anim: node.anim == true) { styled($0) }
        case "time":
            time.font(font).foregroundStyle(toned(node.tone)).monospacedDigit().lineLimit(1).fixedSize()
        case "stat":
            Stat(node: node)
        case "gauge":
            Ring(node: node)
        case "progress":
            Bar(node: node, fraction: fraction)
        case "spark":
            Spark(node: node)
        case "chart":
            BarsOrLine(node: node)
        case "list":
            WidgetList(node: node, now: now)
        case "item":
            WidgetItem(node: node, now: now)
        case "image":
            WidgetImage(node: node)
        case "icon":
            Icon(node: node, font: font)
        case "dot":
            Circle().fill(toned(node.tone, otherwise: .green)).frame(width: 8, height: 8)
        case "link":
            if let link = node.link {
                Linked(link: link) { Text(node.text ?? "").font(font).foregroundStyle(.tint).lineLimit(node.lines ?? 1) }
            }
        case "button":
            ActionButton(node: node)
        case "toggle":
            ActionToggle(node: node)
        case "divider":
            Divider()
        default:
            EmptyView()
        }
    }

    private var fraction: Double {
        let max = node.max ?? 1
        return max > 0 ? min(1, Swift.max(0, (node.value?.number ?? 0) / max)) : 0
    }

    @ViewBuilder private var time: some View {
        let date = Date(timeIntervalSince1970: (node.at ?? 0) / 1000)
        switch node.style {
        case "relative": Text(relative(date))
        case "timer": Text(date, style: .timer)
        case "date": Text(date.formatted(.dateTime.day().month()))
        case "clock": Text(date.formatted(date: .omitted, time: .shortened))
        default: Text(ago(date))
        }
    }

    private func children(inRow: Bool) -> some View {
        // In a row, earlier nodes keep their room first; a node that grows takes what its parent has left.
        ForEach(Array((node.c ?? []).enumerated()), id: \.offset) { i, child in
            AnyView(WidgetTree(node: child, now: now, inRow: inRow))
                .frame(maxWidth: child.grow == true && inRow ? .infinity : nil, maxHeight: child.grow == true && !inRow ? .infinity : nil,
                       alignment: .topLeading)
                .layoutPriority(inRow ? -Double(i) : 0)
        }
    }

    /// "5 min. ago", "in 2 hr.", counted from `now`.
    private func relative(_ date: Date) -> String {
        let format = RelativeDateTimeFormatter()
        format.unitsStyle = .short
        return format.localizedString(for: date, relativeTo: now)
    }

    /// "now", "5 min", "3 h", then the weekday, then the date: short enough for a row.
    private func ago(_ date: Date) -> String {
        let s = now.timeIntervalSince(date)
        if s < 60 { return String(localized: "now") }
        if s < 3600 { return String(localized: "\(Int(s / 60)) min") }
        if s < 86400 { return String(localized: "\(Int(s / 3600)) h") }
        return date.formatted(s < 7 * 86400 ? .dateTime.weekday() : .dateTime.day().month())
    }

    private func styled(_ text: Text) -> some View {
        text
            .font(font)
            .fontWeight(node.weight == "bold" ? .bold : node.weight == "medium" ? .semibold : nil)
            .foregroundStyle(toned(node.tone))
            // In a row, text keeps to one line unless the widget says otherwise, shrinking a little before it truncates.
            .lineLimit(node.lines ?? (inRow ? 1 : nil))
            .minimumScaleFactor(node.lines == 1 || inRow ? 0.85 : 1)
    }

    private var font: Font? { widgetFont(node.size) }
}

func widgetFont(_ size: String?) -> Font? {
    switch size {
    case "xl": .title2
    case "lg": .headline
    case "sm": .footnote
    case "xs": .caption2
    default: nil
    }
}

/// A tone's colour: `muted` is secondary, and with no tone, `otherwise` (the primary colour by default).
func toned(_ tone: String?, otherwise: some ShapeStyle = .primary) -> AnyShapeStyle {
    switch tone {
    case "muted": AnyShapeStyle(.secondary)
    case "accent": AnyShapeStyle(.tint)
    case "good": AnyShapeStyle(.green)
    case "warn": AnyShapeStyle(.orange)
    case "bad": AnyShapeStyle(.red)
    case "blue": AnyShapeStyle(.blue)
    case "purple": AnyShapeStyle(.purple)
    default: AnyShapeStyle(otherwise)
    }
}

/// A tone as a tint: none for the accent, which a tint already is.
func toneColor(_ tone: String?) -> Color? {
    switch tone {
    case "muted": .secondary
    case "good": .green
    case "warn": .orange
    case "bad": .red
    case "blue": .blue
    case "purple": .purple
    default: nil
    }
}

/// Text that shows it changed, with `anim`: a number rolls to its new value, other text crossfades; neither with
/// Reduce Motion on.
private struct Changing<Content: View>: View {
    var text: String
    var number: Double?
    var anim: Bool
    @ViewBuilder var content: (Text) -> Content
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        if anim && !reduceMotion {
            content(Text(text))
                .contentTransition(number.map { .numericText(value: $0) } ?? .interpolate)
                .animation(.smooth(duration: 0.4), value: text)
        } else {
            content(Text(text))
        }
    }
}

private struct Stat: View {
    var node: WidgetNode

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .firstTextBaseline, spacing: 3) {
                Changing(text: node.value?.text ?? "–", number: node.value?.number, anim: node.anim == true) {
                    $0.font(.system(.title, design: .rounded, weight: .semibold)).monospacedDigit()
                        .foregroundStyle(toned(node.tone)).lineLimit(1).minimumScaleFactor(0.6)
                }
                if let unit = node.unit { Text(unit).font(.subheadline.weight(.medium)).foregroundStyle(.secondary) }
            }
            if let label = node.label { Text(label).font(.caption).foregroundStyle(.secondary).lineLimit(1) }
        }
    }
}

private struct Ring: View {
    var node: WidgetNode

    var body: some View {
        let max = node.max ?? 1
        let fraction = max > 0 ? min(1, Swift.max(0, (node.value?.number ?? 0) / max)) : 0
        let size = CGFloat(node.w ?? 46), line = min(5, size / 6)
        ZStack {
            Circle().stroke(.quaternary, lineWidth: line)
            Circle().trim(from: 0, to: fraction)
                .stroke(toned(node.tone, otherwise: .tint), style: StrokeStyle(lineWidth: line, lineCap: .round))
                .rotationEffect(.degrees(-90))
            if node.label != "" {
                Text(node.label ?? "\(Int((fraction * 100).rounded()))%")
                    .font(.caption2.weight(.semibold)).monospacedDigit().lineLimit(1).minimumScaleFactor(0.6)
                    .padding(6)
            }
        }
        .padding(line / 2)
        .frame(width: size, height: size)
        .accessibilityElement()
        .accessibilityLabel(node.label ?? "")
        .accessibilityValue("\(Int((fraction * 100).rounded())) percent")
    }
}

/// A bar filled `fraction` of the way, with its label over it.
private struct Bar: View {
    var node: WidgetNode
    var fraction: Double

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            if let label = node.label { Text(label).font(.caption).foregroundStyle(.secondary).lineLimit(1) }
            GeometryReader { geo in
                ZStack(alignment: .leading) {
                    Capsule().fill(.quaternary)
                    Capsule().fill(toned(node.tone, otherwise: .tint)).frame(width: Swift.max(6, geo.size.width * fraction))
                }
            }
            .frame(width: node.w.map { CGFloat($0) }, height: 6)
        }
        .accessibilityElement()
        .accessibilityValue("\(Int((fraction * 100).rounded())) percent")
    }
}

/// A line through numbers from 0 to `max`, shaded under; a dashed line when there are none.
private struct Spark: View {
    var node: WidgetNode

    var body: some View {
        let values = node.values ?? []
        Canvas { context, size in
            guard !values.isEmpty else {
                var line = Path()
                line.move(to: CGPoint(x: 2, y: size.height / 2))
                line.addLine(to: CGPoint(x: size.width - 2, y: size.height / 2))
                context.stroke(line, with: .style(.quaternary), style: StrokeStyle(lineWidth: 1.5, dash: [2, 3]))
                return
            }
            let data = values.count == 1 ? [values[0], values[0]] : values
            let top = node.max ?? Swift.max(1, data.max() ?? 1)
            let points = data.enumerated().map { i, v in
                CGPoint(x: CGFloat(i) / CGFloat(data.count - 1) * size.width,
                        y: size.height - 1 - CGFloat(Swift.min(Swift.max(v, 0), top) / top) * (size.height - 2))
            }
            var line = Path()
            line.addLines(points)
            var area = line
            area.addLine(to: CGPoint(x: size.width, y: size.height))
            area.addLine(to: CGPoint(x: 0, y: size.height))
            area.closeSubpath()
            let style = toned(node.tone, otherwise: .tint)
            context.fill(area, with: .style(style.opacity(0.2)))
            context.stroke(line, with: .style(style), style: StrokeStyle(lineWidth: 1.4, lineJoin: .round))
        }
        .frame(width: node.w.map { CGFloat($0) }, height: node.h.map { CGFloat($0) } ?? 28)
        .accessibilityElement()
        .accessibilityValue(values.last.map { "\(Int($0))" } ?? "")
    }
}

/// An icon, `w` points across or as big as its font, with its badge (an SVG path) at its bottom trailing corner and
/// the icon cut away around it.
private struct Icon: View {
    var node: WidgetNode
    var font: Font?

    var body: some View {
        let style = toned(node.tone, otherwise: .tint)
        if let w = node.w.map({ CGFloat($0) }) {
            ZStack {
                Image(systemName: symbol(node.name))
                    .resizable()
                    .scaledToFit()
                    .fontWeight(.light)
                    .padding(node.badge == nil ? 0 : w / 12)
                    .foregroundStyle(style)
                    .mask {
                        if node.badge != nil {
                            Rectangle()
                                .overlay(Circle().frame(width: w * 14 / 24, height: w * 14 / 24).offset(x: w * 9 / 24, y: w * 9 / 24).blendMode(.destinationOut))
                                .compositingGroup()
                        } else {
                            Rectangle()
                        }
                    }
                if let badge = node.badge {
                    SVGShape(badge).fill(style)
                        .frame(width: w * 11 / 24, height: w * 11 / 24)
                        .offset(x: w * 9 / 24, y: w * 9 / 24)
                }
            }
            .frame(width: w, height: w)
        } else {
            Image(systemName: symbol(node.name))
                .font(node.size.map { _ in font } ?? nil)
                .foregroundStyle(style)
        }
    }
}

private struct BarsOrLine: View {
    var node: WidgetNode

    private struct Point: Identifiable {
        var id: Int
        /// Its place, as a category of its own, so labels that repeat (M T W T F S S) stay apart.
        var x: String { "\(id)" }
        var y: Double
    }

    var body: some View {
        let points = (node.values ?? []).enumerated().map { Point(id: $0.offset, y: $0.element) }
        let labels = node.labels ?? []
        return Chart(points) { point in
            if node.kind == "line" {
                LineMark(x: .value("x", point.x), y: .value("y", point.y))
            } else {
                BarMark(x: .value("x", point.x), y: .value("y", point.y))
            }
        }
        .chartYAxis(.hidden)
        .chartXAxis {
            AxisMarks { value in
                AxisValueLabel {
                    let i = Int(value.as(String.self) ?? "") ?? -1
                    Text(labels.indices.contains(i) ? labels[i] : "")
                }
            }
        }
        .foregroundStyle(toned(node.tone, otherwise: .tint))
        .frame(height: node.h.map { CGFloat($0) } ?? 80)
    }
}

private struct WidgetList: View {
    var node: WidgetNode
    var now: Date

    var body: some View {
        let items = node.c ?? []
        if items.isEmpty {
            Text(node.empty ?? String(localized: "Nothing here")).foregroundStyle(.secondary)
        } else {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(items.enumerated()), id: \.offset) { i, item in
                    if i > 0 { Divider().padding(.leading, item.icon == nil ? 0 : 30) }
                    WidgetTree(node: item, now: now).padding(.vertical, 6)
                }
            }
        }
    }
}

private struct WidgetItem: View {
    var node: WidgetNode
    var now: Date

    var body: some View {
        let row = HStack(spacing: 10) {
            if let icon = node.icon {
                Image(systemName: symbol(icon)).foregroundStyle(toned(node.tone, otherwise: .tint)).frame(width: 20)
            }
            VStack(alignment: .leading, spacing: 1) {
                Text(node.title ?? "").lineLimit(1).foregroundStyle(.primary)
                if let subtitle = node.subtitle { Text(subtitle).font(.caption).foregroundStyle(.secondary).lineLimit(1) }
            }
            Spacer(minLength: 4)
            if let at = node.at {
                WidgetTree(node: WidgetNode(t: "time", size: "sm", tone: "muted", at: at), now: now)
            } else if let trailing = node.trailing {
                Text(trailing).font(.callout).foregroundStyle(.secondary).monospacedDigit().lineLimit(1)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(.rect)
        if let link = node.link { Linked(link: link) { row } } else { row }
    }
}

private struct WidgetImage: View {
    var node: WidgetNode

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: node.round == true ? 999 : 8, style: .continuous)
        let w = node.w.map { CGFloat($0) }, h = node.h.map { CGFloat($0) }
        Group {
            #if WIDGET
            if let image = node.url.flatMap(WidgetImages.load) {
                image.resizable().scaledToFill()
            } else {
                Rectangle().fill(.quaternary)
            }
            #else
            AsyncImage(url: node.url.flatMap(URL.init(string:))) { image in
                image.resizable().scaledToFill()
            } placeholder: {
                Rectangle().fill(.quaternary)
            }
            #endif
        }
        .frame(width: w, height: h ?? (w == nil ? 120 : w))
        .frame(maxWidth: w == nil ? .infinity : nil)
        .clipShape(shape)
    }
}

/// A node that opens a link when tapped: a link of its own on a home-screen widget, or the host's.
private struct Linked<Content: View>: View {
    var link: WidgetLink
    @ViewBuilder var content: Content
    @Environment(\.widgetHost) private var host

    var body: some View {
        #if WIDGET
        if let url = link.url(app: host.app, scope: host.scope) {
            Link(destination: url) { content }
        } else {
            content
        }
        #else
        if let open = host.open {
            Button { open(link) } label: { content.contentShape(.rect) }.buttonStyle(.plain)
        } else {
            content
        }
        #endif
    }
}

private struct ActionButton: View {
    var node: WidgetNode
    @Environment(\.widgetHost) private var host
    @State private var running = false
    @State private var error: String?

    var body: some View {
        let label = Label {
            Text(node.label ?? node.text ?? "")
        } icon: {
            if let icon = node.icon { Image(systemName: symbol(icon)) }
        }
        #if WIDGET
        Button(intent: RunOperation(app: host.app, op: node.op ?? "", input: node.input ?? .object([:]), scope: host.scope)) { label }
            .buttonStyle(.bordered)
            .tint(toneColor(node.tone))
        #else
        Button(role: node.tone == "bad" ? .destructive : nil) {
            guard let run = host.run, let op = node.op, !running else { return }
            running = true
            Task {
                error = await run(op, node.input ?? .object([:]))
                running = false
            }
        } label: {
            label.opacity(running ? 0 : 1).overlay { if running { ProgressView().controlSize(.small) } }
        }
        .buttonStyle(.bordered)
        .controlSize(.small)
        .tint(toneColor(node.tone))
        .disabled(host.run == nil || node.op == nil)
        .alert("Couldn’t Do That", isPresented: Binding(get: { error != nil }, set: { if !$0 { error = nil } })) {
            Button("OK") { error = nil }
        } message: {
            Text(error ?? "")
        }
        #endif
    }
}

private struct ActionToggle: View {
    var node: WidgetNode
    @Environment(\.widgetHost) private var host
    /// What it shows while its operation runs, before the widget redraws.
    @State private var pending: Bool?
    @State private var error: String?

    var body: some View {
        let on = pending ?? node.on ?? false
        #if WIDGET
        Toggle(isOn: on, intent: ToggleOperation(app: host.app, op: node.op ?? "", input: node.input ?? .object([:]), scope: host.scope)) {
            Text(node.label ?? "").lineLimit(1)
        }
        .tint(.accentColor)
        #else
        Toggle(isOn: Binding(get: { on }, set: { value in
            guard let run = host.run, let op = node.op else { return }
            pending = value
            Task {
                error = await run(op, (node.input ?? .object([:])).merging("on", .bool(value)))
                pending = nil
            }
        })) {
            Text(node.label ?? "").lineLimit(1)
        }
        .disabled(host.run == nil || node.op == nil)
        .onChange(of: node.on) { pending = nil }
        .alert("Couldn’t Do That", isPresented: Binding(get: { error != nil }, set: { if !$0 { error = nil } })) {
            Button("OK") { error = nil }
        } message: {
            Text(error ?? "")
        }
        #endif
    }
}
