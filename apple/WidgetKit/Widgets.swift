import AppIntents
import SwiftUI
import WidgetKit

// Home-screen and desktop widgets: any widget on the user's network, chosen
// when the widget is added, drawn by the same renderer as the app.

struct WidgetChoice: AppEntity {
    let id: String  // the widget's MCP tool
    let title: String

    static let typeDisplayRepresentation: TypeDisplayRepresentation = "Widget"
    static let defaultQuery = WidgetChoices()
    var displayRepresentation: DisplayRepresentation { DisplayRepresentation(title: "\(title)") }
}

struct WidgetChoices: EntityQuery {
    func entities(for identifiers: [String]) async throws -> [WidgetChoice] {
        try await suggestedEntities().filter { identifiers.contains($0.id) }
    }

    // Every tool marked as a widget, named by the title it draws with.
    func suggestedEntities() async throws -> [WidgetChoice] {
        guard let client = await Network.client("The One App widget"),
              let tools = try await client.request("tools/list")["tools"] as? [[String: Any]] else { return [] }
        var choices: [WidgetChoice] = []
        for tool in tools where ((tool["_meta"] as? [String: Any])?["dev.theoneapp/widget"] as? Bool) == true {
            guard let name = tool["name"] as? String else { continue }
            let drawn = try? await client.request("tools/call", ["name": name, "arguments": [:] as [String: Any]])
            choices.append(WidgetChoice(id: name, title: drawn.flatMap(Description.init(result:))?.title ?? name))
        }
        return choices
    }
}

struct ChooseWidget: WidgetConfigurationIntent {
    static let title: LocalizedStringResource = "Choose a widget"
    static let description = IntentDescription("Show a widget from your devices.")
    @Parameter(title: "Widget") var widget: WidgetChoice?
}

struct Entry: TimelineEntry {
    var date = Date()
    var description: Description?
    var message = ""
}

struct Provider: AppIntentTimelineProvider {
    func placeholder(in context: Context) -> Entry { Entry() }

    func snapshot(for configuration: ChooseWidget, in context: Context) async -> Entry {
        await draw(configuration, context.displaySize)
    }

    func timeline(for configuration: ChooseWidget, in context: Context) async -> Timeline<Entry> {
        let entry = await draw(configuration, context.displaySize)
        let next = entry.description.map { Date(timeIntervalSince1970: $0.next / 1000) } ?? Date().addingTimeInterval(15 * 60)
        return Timeline(entries: [entry], policy: .after(max(next, Date().addingTimeInterval(60))))
    }

    // Without a choice, the first widget on the network.
    private func draw(_ configuration: ChooseWidget, _ size: CGSize) async -> Entry {
        guard let client = await Network.client("The One App widget") else { return Entry(message: "None of your devices answered.") }
        var tool = configuration.widget?.id
        if tool == nil {
            tool = (try? await WidgetChoices().suggestedEntities())?.first?.id
        }
        guard let tool else { return Entry(message: "No widgets yet.") }
        do {
            let result = try await client.request("tools/call", ["name": tool, "arguments": ["w": size.width, "h": size.height]])
            guard let d = Description(result: result) else { return Entry(message: "This widget couldn't be drawn.") }
            return Entry(description: d)
        } catch {
            return Entry(message: error.localizedDescription)
        }
    }
}

struct EntryView: View {
    let entry: Entry

    var body: some View {
        Group {
            if let d = entry.description {
                WidgetView(description: d, ctx: WidgetContext(homeScreen: true))
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            } else {
                Text(entry.message).font(.system(size: 13)).foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .containerBackground(.fill.tertiary, for: .widget)
    }
}

@main
struct OneWidgets: Widget {
    var body: some WidgetConfiguration {
        AppIntentConfiguration(kind: "widget", intent: ChooseWidget.self, provider: Provider()) { entry in
            EntryView(entry: entry)
        }
        .configurationDisplayName("The One App")
        .description("A widget from your devices.")
        .supportedFamilies([.systemSmall, .systemMedium, .systemLarge, .systemExtraLarge])
    }
}
