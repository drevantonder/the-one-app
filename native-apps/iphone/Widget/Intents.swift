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
