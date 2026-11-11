import SwiftUI

/// A widget's button in the menu bar: the app runs its tool. (A home-screen widget's are intents: iphone/Widget/Actions.swift.)
struct ActionButton: View {
    var node: WidgetNode
    @Environment(\.widgetHost) private var host

    var body: some View {
        Button { run(host, node, input: node.input ?? .object([:])) } label: {
            Label {
                Text(node.label ?? node.text ?? "")
            } icon: {
                if let icon = node.icon { Glyph(name: icon, size: 16) }
            }
        }
        .buttonStyle(.bordered)
        .tint(toneColor(node.tone))
    }
}

/// A widget's toggle: its tool gets the toggle's new value as `on`.
struct ActionToggle: View {
    var node: WidgetNode
    @Environment(\.widgetHost) private var host

    var body: some View {
        Toggle(isOn: Binding(get: { node.on ?? false }, set: { on in
            run(host, node, input: (node.input ?? .object([:])).merging("on", .bool(on)))
        })) {
            Text(node.label ?? "").lineLimit(1)
        }
        .toggleStyle(.switch)
        .controlSize(.mini)
    }
}

private func run(_ host: WidgetHost, _ node: WidgetNode, input: JSONValue) {
    chose(["do": "run", "app": host.app, "tool": node.tool ?? "", "input": (try? JSONSerialization.jsonObject(with: input.data, options: .fragmentsAllowed)) ?? [:]])
}
