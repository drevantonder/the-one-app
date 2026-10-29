import CoreGraphics
import Foundation

// A widget as plugins return it (package sdk/widget): the PTC App's widget
// description v1 plus gauge, spark, icon and dot. Unknown node types and
// fields are skipped.

enum JSON: Codable {
    case null, bool(Bool), number(Double), string(String), array([JSON]), object([String: JSON])

    init(from decoder: Decoder) throws {
        let value = try decoder.singleValueContainer()
        if value.decodeNil() { self = .null }
        else if let v = try? value.decode(Bool.self) { self = .bool(v) }
        else if let v = try? value.decode(Double.self) { self = .number(v) }
        else if let v = try? value.decode(String.self) { self = .string(v) }
        else if let v = try? value.decode([JSON].self) { self = .array(v) }
        else { self = .object(try value.decode([String: JSON].self)) }
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

    var any: Any {
        switch self {
        case .null: return NSNull()
        case .bool(let v): return v
        case .number(let v): return v
        case .string(let v): return v
        case .array(let v): return v.map(\.any)
        case .object(let v): return v.mapValues(\.any)
        }
    }
}

/// A child that fails to decode is dropped instead of failing the whole widget.
struct Skippable<T: Decodable>: Decodable {
    let value: T?
    init(from decoder: Decoder) throws { value = try? T(from: decoder) }
    init(value: T?) { self.value = value }
}

struct WidgetAction: Decodable {
    let op: String
    let input: [String: JSON]?
}

struct Stack: Decodable {
    let t: String
    let c: [Skippable<Node>]
    let gap: CGFloat?, pad: CGFloat?, w: CGFloat?, grow: Bool?, dim: Bool?
    let align: String?, justify: String?
    var vertical: Bool { t == "col" }
    var children: [Node] { c.compactMap(\.value) }

    /// A link wraps its children like a column.
    static func column(_ children: [Node]) -> Stack {
        Stack(t: "col", c: children.map { Skippable(value: $0) }, gap: nil, pad: nil, w: nil, grow: nil, dim: nil, align: nil, justify: nil)
    }
}

struct LinkNode: Decodable {
    let to: String
    let c: [Skippable<Node>]
    var children: [Node] { c.compactMap(\.value) }
}

struct TextNode: Decodable {
    let text: String
    let size: String?, weight: String?, tone: String?
    let lines: Int?
    let align: String?
    let anim: String?
}

struct TimeNode: Decodable {
    let at: Double
    let style: String
    let size: String?, weight: String?, tone: String?
}

struct ImageNode: Decodable {
    let src: String
    let w: CGFloat?, h: CGFloat?
    let round: Bool?, radius: CGFloat?
    let alt: String?
}

struct Item: Decodable {
    let title: String
    let sub: String?
    let end: Skippable<Node>?
    let img: ImageNode?
    let link: String?
}

struct StatNode: Decodable { let value: String; let label: String?; let anim: String? }
struct ChartPoint: Decodable { let value: Double; let label: String? }
struct ChartNode: Decodable { let kind: String; let data: [ChartPoint]; let hl: Int?; let h: CGFloat?; let grow: Bool? }
struct ButtonNode: Decodable { let label: String; let action: WidgetAction; let primary: Bool? }
struct ToggleNode: Decodable { let label: String; let on: Bool; let action: WidgetAction }
struct ProgressNode: Decodable { let value: Double; let label: String?; let w: CGFloat?; let color: String? }
struct SpacerNode: Decodable { let size: CGFloat? }
struct ListNode: Decodable { let c: [Skippable<Item>]; let grow: Bool?; var items: [Item] { c.compactMap(\.value) } }
struct GaugeNode: Decodable { let value: Double; let color: String?; let size: CGFloat? }
struct SparkNode: Decodable { let data: [Double]; let max: Double?; let color: String?; let w: CGFloat; let h: CGFloat }
struct IconNode: Decodable { let path: String; let stroke: Bool?; let size: CGFloat?; let badge: String? }
struct DotNode: Decodable { let color: String? }

indirect enum Node: Decodable {
    case stack(Stack), link(LinkNode), text(TextNode), time(TimeNode), image(ImageNode), spacer(SpacerNode)
    case stat(StatNode), list(ListNode), chart(ChartNode), button(ButtonNode), toggle(ToggleNode), progress(ProgressNode)
    case gauge(GaugeNode), spark(SparkNode), icon(IconNode), dot(DotNode), unknown

    private enum Key: String, CodingKey { case t }

    init(from decoder: Decoder) throws {
        switch try decoder.container(keyedBy: Key.self).decode(String.self, forKey: .t) {
        case "row", "col": self = .stack(try Stack(from: decoder))
        case "link": self = .link(try LinkNode(from: decoder))
        case "text": self = .text(try TextNode(from: decoder))
        case "time": self = .time(try TimeNode(from: decoder))
        case "img": self = .image(try ImageNode(from: decoder))
        case "spacer": self = .spacer(try SpacerNode(from: decoder))
        case "stat": self = .stat(try StatNode(from: decoder))
        case "list": self = .list(try ListNode(from: decoder))
        case "chart": self = .chart(try ChartNode(from: decoder))
        case "button": self = .button(try ButtonNode(from: decoder))
        case "toggle": self = .toggle(try ToggleNode(from: decoder))
        case "progress": self = .progress(try ProgressNode(from: decoder))
        case "gauge": self = .gauge(try GaugeNode(from: decoder))
        case "spark": self = .spark(try SparkNode(from: decoder))
        case "icon": self = .icon(try IconNode(from: decoder))
        case "dot": self = .dot(try DotNode(from: decoder))
        default: self = .unknown
        }
    }
}

struct Description: Decodable {
    let v: Int
    let app: String
    let widget: String
    let title: String
    let at: Double
    let next: Double
    let link: String?
    let root: Node

    /// Decodes a widget tool's result, as returned over MCP.
    init?(result: [String: Any]) {
        guard let content = result["structuredContent"],
              let data = try? JSONSerialization.data(withJSONObject: content),
              let d = try? JSONDecoder().decode(Description.self, from: data) else { return nil }
        self = d
    }

    /// The MCP tool for one of the widget's plugin's operations.
    func tool(_ op: String) -> String { app == "one" ? op : "\(app)_\(op)" }
}
