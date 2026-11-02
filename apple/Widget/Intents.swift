import AppIntents
import Foundation

/// What runs a home-screen widget's operations: the app sets it, since only the app reaches a device.
@MainActor
enum WidgetActions {
    static var run: (@MainActor @Sendable (_ app: String, _ op: String, _ input: Data, _ scope: String?) async throws -> Void)?
}

/// A widget's button. As a Live Activity intent it runs in the app's process, which the system starts if it must.
struct RunOperation: LiveActivityIntent {
    static let title: LocalizedStringResource = "Run a Widget’s Button"
    static let isDiscoverable = false

    @Parameter(title: "App") var app: String
    @Parameter(title: "Operation") var op: String
    /// Its input, as JSON.
    @Parameter(title: "Input") var input: String
    @Parameter(title: "Page") var scope: String?

    init() {}

    init(app: String, op: String, input: JSONValue, scope: String?) {
        self.app = app
        self.op = op
        self.input = String(decoding: input.data, as: UTF8.self)
        self.scope = scope
    }

    @MainActor
    func perform() async throws -> some IntentResult {
        try await WidgetActions.run?(app, op, Data(input.utf8), scope)
        return .result()
    }
}

/// A widget's toggle: its operation gets the toggle's new value as `on`.
struct ToggleOperation: SetValueIntent, LiveActivityIntent {
    static let title: LocalizedStringResource = "Run a Widget’s Toggle"
    static let isDiscoverable = false

    @Parameter(title: "App") var app: String
    @Parameter(title: "Operation") var op: String
    @Parameter(title: "Input") var input: String
    @Parameter(title: "Page") var scope: String?
    @Parameter(title: "On") var value: Bool

    init() {}

    init(app: String, op: String, input: JSONValue, scope: String?) {
        self.app = app
        self.op = op
        self.input = String(decoding: input.data, as: UTF8.self)
        self.scope = scope
    }

    @MainActor
    func perform() async throws -> some IntentResult {
        let base = (try? JSONDecoder().decode(JSONValue.self, from: Data(input.utf8))) ?? .object([:])
        try await WidgetActions.run?(app, op, base.merging("on", .bool(value)).data, scope)
        return .result()
    }
}
