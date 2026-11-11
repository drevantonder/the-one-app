import Foundation

/// Any JSON value: a tool's input, as a widget gives it.
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
