import SwiftUI

/// A rendered widget, as `GET /v1/widgets` gives it.
struct WidgetInfo: Codable, Sendable, Equatable, Identifiable {
    var app: String
    var widget: String
    var title: String?
    var root: WidgetNode

    var id: String { "\(app).\(widget)" }
}

/// A node of a widget's tree (SPEC.md "Widgets"). Fields a renderer doesn't know are skipped.
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

    /// Its most prominent text (the biggest, then the first that isn't muted): what a one-line place shows.
    var headline: String? {
        func texts(_ n: WidgetNode) -> [WidgetNode] { n.t == "text" ? [n] : (n.c ?? []).flatMap(texts) }
        let rank = ["sm": 0, "md": 1, "lg": 2, "xl": 3]
        return texts(self).enumerated().max { a, b in
            let ka = (rank[a.element.size ?? "md"] ?? 1, a.element.tone == "muted" ? 0 : 1, -a.offset)
            let kb = (rank[b.element.size ?? "md"] ?? 1, b.element.tone == "muted" ? 0 : 1, -b.offset)
            return ka < kb
        }?.element.text
    }
}

/// Draws a widget's tree. Text of the default size takes the surrounding font, so each place sets its own.
struct WidgetTree: View {
    var node: WidgetNode
    /// What `time` nodes count from: a home-screen widget passes its timeline entry's date.
    var now: Date = .now
    var inRow = false

    var body: some View {
        switch node.t {
        case "col":
            VStack(alignment: .leading, spacing: node.gap.map { CGFloat($0) } ?? 4) { children(inRow: false) }
        case "row":
            HStack(alignment: .firstTextBaseline, spacing: node.gap.map { CGFloat($0) } ?? 6) { children(inRow: true) }
        case "text":
            styled(Text(node.text ?? ""))
        case "time":
            styled(Text(ago(Date(timeIntervalSince1970: (node.at ?? 0) / 1000)))).fixedSize()
        default:
            EmptyView()
        }
    }

    private func children(inRow: Bool) -> some View {
        // In a row, earlier nodes keep their room first.
        ForEach(Array((node.c ?? []).enumerated()), id: \.offset) { i, child in
            AnyView(WidgetTree(node: child, now: now, inRow: inRow)).layoutPriority(inRow ? -Double(i) : 0)
        }
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
            .foregroundStyle(node.tone == "muted" ? AnyShapeStyle(.secondary) : AnyShapeStyle(.primary))
            // In a row, text keeps to one line unless the widget says otherwise.
            .lineLimit(node.lines ?? (inRow ? 1 : nil))
    }

    private var font: Font? {
        switch node.size {
        case "xl": .title2
        case "lg": .headline
        case "sm": .caption
        default: nil
        }
    }
}
