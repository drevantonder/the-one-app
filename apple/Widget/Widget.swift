import SwiftUI
import WidgetKit

@main
struct Widgets: WidgetBundle {
    var body: some Widget { SpaceWidget() }
}

/// The space's first widget, drawn as the app draws it.
struct SpaceWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "widget", provider: Provider()) { entry in
            EntryView(entry: entry)
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName(appName)
        .description("What’s new in your space.")
        .supportedFamilies([.systemSmall, .systemMedium, .accessoryRectangular])
    }
}

struct Entry: TimelineEntry {
    var date = Date.now
    var widget: WidgetCache.Item?
}

/// The app reloads the timeline whenever its widgets change; between reloads, times count on from each entry.
struct Provider: TimelineProvider {
    func placeholder(in context: Context) -> Entry {
        Entry(widget: .init(title: appName, icon: "app", root: WidgetNode(t: "col", c: [
            WidgetNode(t: "text", text: "MacBook Air", size: "sm", weight: "medium"),
            WidgetNode(t: "text", text: "Deployed the new build to every device.", lines: 3),
        ])))
    }

    func getSnapshot(in context: Context, completion: @escaping (Entry) -> Void) {
        completion(context.isPreview ? placeholder(in: context) : Entry(widget: WidgetCache.load()?.widgets.first))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<Entry>) -> Void) {
        let widget = WidgetCache.load()?.widgets.first
        let entries = (0..<36).map { Entry(date: .now.addingTimeInterval(Double($0) * 300), widget: widget) }
        completion(Timeline(entries: entries, policy: .atEnd))
    }
}

struct EntryView: View {
    var entry: Entry
    @Environment(\.widgetFamily) private var family

    private var accent: Color {
        Color(brandHex: Bundle.main.object(forInfoDictionaryKey: "BrandAccent") as? String ?? "")
    }

    var body: some View {
        if let widget = entry.widget {
            VStack(alignment: .leading, spacing: 6) {
                if family != .accessoryRectangular {
                    HStack(spacing: 6) {
                        Image("Logo")
                            .resizable()
                            .frame(width: 18, height: 18)
                        Text(widget.title)
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(accent)
                            .lineLimit(1)
                    }
                }
                WidgetTree(node: widget.root, now: entry.date)
                    .font(family == .systemSmall ? .footnote : .subheadline)
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        } else {
            VStack(spacing: 8) {
                Image("Logo")
                    .resizable()
                    .frame(width: 44, height: 44)
                Text("Open \(appName) to see your space.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
        }
    }
}
