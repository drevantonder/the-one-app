import AppIntents
import Foundation

#if !WIDGET
/// The app's Rust (src/platform/iphone.rs `one_ios`), which runs a widget's tool as the user, then draws the widgets again.
@_silgen_name("one_ios") private func oneIOS(_ kind: UnsafePointer<CChar>, _ json: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?
@_silgen_name("one_ios_free") private func oneIOSFree(_ s: UnsafeMutablePointer<CChar>?)
#endif

/// What runs a home-screen widget's tools: the app, which reaches the user's devices, in its own process (each tool's
/// intent is a Live Activity intent, so the system starts the app for it if it must). The widget only names them.
enum WidgetActions {
    static func run(_ app: String, _ tool: String, _ input: Data) async throws {
        #if !WIDGET
        let asked: [String: Any] = ["app": app, "tool": tool, "input": (try? JSONSerialization.jsonObject(with: input, options: .fragmentsAllowed)) ?? [:]]
        let error = await Task.detached { () -> String? in
            let json = (try? JSONSerialization.data(withJSONObject: asked)).map { String(decoding: $0, as: UTF8.self) } ?? "{}"
            let out = "call".withCString { k in json.withCString { j in oneIOS(k, j) } }
            defer { oneIOSFree(out) }
            let said = out.flatMap { try? JSONSerialization.jsonObject(with: Data(String(cString: $0).utf8)) } as? [String: Any]
            return said?["error"] as? String
        }.value
        if let error { throw WidgetError(text: error) }
        #endif
    }
}

struct WidgetError: LocalizedError {
    var text: String
    var errorDescription: String? { text }
}

/// A widget's button. As a Live Activity intent it runs in the app's process, which the system starts if it must.
struct RunTool: LiveActivityIntent {
    static let title: LocalizedStringResource = "Run a Widget’s Button"
    static let isDiscoverable = false

    @Parameter(title: "App") var app: String
    @Parameter(title: "Tool") var tool: String
    /// Its input, as JSON.
    @Parameter(title: "Input") var input: String
    @Parameter(title: "Page") var scope: String?

    init() {}

    init(app: String, tool: String, input: JSONValue, scope: String?) {
        self.app = app
        self.tool = tool
        self.input = String(decoding: input.data, as: UTF8.self)
        self.scope = scope
    }

    @MainActor
    func perform() async throws -> some IntentResult {
        try await WidgetActions.run(app, tool, Data(input.utf8))
        return .result()
    }
}

/// A widget's toggle: its tool gets the toggle's new value as `on`.
struct ToggleTool: SetValueIntent, LiveActivityIntent {
    static let title: LocalizedStringResource = "Run a Widget’s Toggle"
    static let isDiscoverable = false

    @Parameter(title: "App") var app: String
    @Parameter(title: "Tool") var tool: String
    @Parameter(title: "Input") var input: String
    @Parameter(title: "Page") var scope: String?
    @Parameter(title: "On") var value: Bool

    init() {}

    init(app: String, tool: String, input: JSONValue, scope: String?) {
        self.app = app
        self.tool = tool
        self.input = String(decoding: input.data, as: UTF8.self)
        self.scope = scope
    }

    @MainActor
    func perform() async throws -> some IntentResult {
        let base = (try? JSONDecoder().decode(JSONValue.self, from: Data(input.utf8))) ?? .object([:])
        try await WidgetActions.run(app, tool, base.merging("on", .bool(value)).data)
        return .result()
    }
}

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
