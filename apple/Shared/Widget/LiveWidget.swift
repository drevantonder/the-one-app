import SwiftUI

/// A widget tool's latest result, fetched from a node, with its buttons and
/// toggles running on the node.
@MainActor
final class LiveWidget: ObservableObject, Identifiable {
    let tool: String
    @Published var description: Description?
    @Published var error: String?
    var changed: (() -> Void)?
    private let client: MCPClient

    nonisolated var id: String { tool }

    init(tool: String, client: MCPClient, result: [String: Any]? = nil) {
        self.tool = tool
        self.client = client
        if let result { description = Description(result: result) }
    }

    func show(_ result: [String: Any]) {
        guard let d = Description(result: result) else {
            error = (result["content"] as? [[String: Any]])?.first?["text"] as? String ?? "This widget couldn't be drawn."
            return
        }
        description = d
        error = nil
        changed?()
    }

    func load() async {
        do {
            show(try await client.request("tools/call", ["name": tool, "arguments": [:] as [String: Any]]))
        } catch {
            self.error = error.localizedDescription
        }
    }

    var context: WidgetContext {
        WidgetContext(run: { [weak self] action in try await self?.act(action) })
    }

    private func act(_ action: WidgetAction) async throws {
        guard let d = description else { return }
        let input = (action.input ?? [:]).mapValues(\.any)
        let result = try await client.request("tools/call", ["name": d.tool(action.op), "arguments": input])
        if result["isError"] as? Bool == true {
            throw MCPError.rpc((result["content"] as? [[String: Any]])?.first?["text"] as? String ?? "That didn't work.")
        }
        await load()
    }
}

struct LiveWidgetView: View {
    @ObservedObject var widget: LiveWidget

    var body: some View {
        if let d = widget.description {
            WidgetView(description: d, ctx: widget.context)
        } else if let error = widget.error {
            Text(error).font(.system(size: 12)).foregroundStyle(.secondary)
        } else {
            ProgressView().controlSize(.small)
        }
    }
}
