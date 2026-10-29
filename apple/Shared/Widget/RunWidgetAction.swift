import AppIntents
import Foundation
import WidgetKit

/// A widget's button or toggle on a home screen: runs its tool on the
/// network, then draws the widgets again.
struct RunWidgetAction: AppIntent {
    static let title: LocalizedStringResource = "Run a widget's action"
    static let isDiscoverable = false

    @Parameter(title: "Tool") var tool: String
    @Parameter(title: "Input") var input: String

    init() {}

    init(tool: String, input: [String: JSON]?) {
        self.tool = tool
        self.input = (try? JSONEncoder().encode(input ?? [:])).map { String(decoding: $0, as: UTF8.self) } ?? "{}"
    }

    func perform() async throws -> some IntentResult {
        guard let client = await Network.client("The One App widget") else { throw MCPError.rpc("None of your devices answered.") }
        let args = (try? JSONSerialization.jsonObject(with: Data(input.utf8))) as? [String: Any] ?? [:]
        _ = try await client.request("tools/call", ["name": tool, "arguments": args])
        WidgetCenter.shared.reloadAllTimelines()
        return .result()
    }
}
