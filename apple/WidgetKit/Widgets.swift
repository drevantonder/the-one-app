import AppIntents
import SwiftUI
import WidgetKit

// Home-screen, desktop and lock-screen widgets: any widget on the user's
// network, chosen when the widget is added, drawn by the same renderer as the
// app. The lock screen shows the first line of its text, as the menu bar does.

struct WidgetChoice: AppEntity {
    let id: String  // the widget's MCP tool
    let title: String

    static let typeDisplayRepresentation: TypeDisplayRepresentation = "Widget"
    static let defaultQuery = WidgetChoices()
    var displayRepresentation: DisplayRepresentation { DisplayRepresentation(title: "\(title)") }
}

struct WidgetChoices: EntityQuery {
    // A chosen widget is known by its tool alone, so a home-screen widget
    // finds its choice again without asking the network.
    func entities(for identifiers: [String]) async throws -> [WidgetChoice] {
        identifiers.map { WidgetChoice(id: $0, title: UserDefaults.standard.string(forKey: "title." + $0) ?? $0) }
    }

    // Every tool marked as a widget, named by the title it draws with; the
    // tools are asked all at once.
    func suggestedEntities() async throws -> [WidgetChoice] {
        guard let client = await Network.client("The One App widget"),
              let tools = try await client.request("tools/list")["tools"] as? [[String: Any]] else { return [] }
        let names = tools.compactMap { tool -> String? in
            ((tool["_meta"] as? [String: Any])?["dev.theoneapp/widget"] as? Bool) == true ? tool["name"] as? String : nil
        }
        let titles = await withTaskGroup(of: (String, String?).self) { group in
            for name in names {
                group.addTask {
                    let drawn = try? await client.request("tools/call", ["name": name, "arguments": [:] as [String: Any]])
                    return (name, drawn.flatMap(Description.init(result:))?.title)
                }
            }
            return await group.reduce(into: [String: String]()) { $0[$1.0] = $1.1 }
        }
        return names.map { name in
            let title = titles[name] ?? name
            UserDefaults.standard.set(title, forKey: "title." + name)
            return WidgetChoice(id: name, title: title)
        }
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
    var line = ""
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
            let text = (result["content"] as? [[String: Any]])?.first?["text"] as? String ?? ""
            return Entry(description: d, line: text.split(separator: "\n", maxSplits: 1).first.map(String.init) ?? "")
        } catch {
            return Entry(message: error.localizedDescription)
        }
    }
}

struct EntryView: View {
    let entry: Entry
    @Environment(\.widgetFamily) private var family

    var body: some View {
        Group {
            #if os(iOS)
            if family == .accessoryInline {
                Text(entry.description == nil ? entry.message : entry.line)
            } else if family == .accessoryRectangular {
                Text(entry.description == nil ? entry.message : entry.line)
                    .font(.headline).lineLimit(3)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
            } else {
                content
            }
            #else
            content
            #endif
        }
        .containerBackground(.fill.tertiary, for: .widget)
    }

    @ViewBuilder private var content: some View {
        Group {
            if let d = entry.description {
                WidgetView(description: d, ctx: WidgetContext(homeScreen: true))
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            } else {
                Text(entry.message).font(.system(size: 13)).foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
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
        .supportedFamilies(families)
    }

    private var families: [WidgetFamily] {
        #if os(iOS)
        [.systemSmall, .systemMedium, .systemLarge, .systemExtraLarge, .accessoryInline, .accessoryRectangular]
        #else
        [.systemSmall, .systemMedium, .systemLarge, .systemExtraLarge]
        #endif
    }
}
