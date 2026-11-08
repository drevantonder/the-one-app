import AppIntents
import SwiftUI

/// A home-screen widget's button: an intent, which is all a widget can run. (The Mac's are mac/Sources/OneNativeMac/Actions.swift.)
struct ActionButton: View {
    var node: WidgetNode
    @Environment(\.widgetHost) private var host

    var body: some View {
        let label = Label {
            Text(node.label ?? node.text ?? "")
        } icon: {
            if let icon = node.icon { Glyph(name: icon, size: 16) }
        }
        Button(intent: RunTool(app: host.app, tool: node.tool ?? "", input: node.input ?? .object([:]), scope: host.scope)) { label }
            .buttonStyle(.bordered)
            .tint(toneColor(node.tone))
    }
}

struct ActionToggle: View {
    var node: WidgetNode
    @Environment(\.widgetHost) private var host

    var body: some View {
        let on = node.on ?? false
        Toggle(isOn: on, intent: ToggleTool(app: host.app, tool: node.tool ?? "", input: node.input ?? .object([:]), scope: host.scope)) {
            Text(node.label ?? "").lineLimit(1)
        }
        .tint(.accentColor)
    }
}
