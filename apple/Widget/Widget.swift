import SwiftUI
import WidgetKit

@main
struct Widgets: WidgetBundle {
    var body: some Widget { NewestPost() }
}

struct NewestPost: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "newest", provider: Provider()) { entry in
            NewestView(newest: entry.newest)
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("Newest Post")
        .description("The newest post on your space’s board.")
        .supportedFamilies([.systemSmall, .systemMedium, .accessoryRectangular])
    }
}

struct Entry: TimelineEntry {
    var date = Date.now
    var newest: Newest?
}

/// The app reloads the widget whenever the newest post changes.
struct Provider: TimelineProvider {
    func placeholder(in context: Context) -> Entry {
        Entry(newest: Newest(name: "MacBook Air", text: "Deployed the new build to every device.", topic: "", time: .now))
    }

    func getSnapshot(in context: Context, completion: @escaping (Entry) -> Void) {
        completion(context.isPreview ? placeholder(in: context) : Entry(newest: Newest.load()))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<Entry>) -> Void) {
        completion(Timeline(entries: [Entry(newest: Newest.load())], policy: .never))
    }
}

struct NewestView: View {
    var newest: Newest?
    @Environment(\.widgetFamily) private var family

    private var accent: Color {
        Color(brandHex: Bundle.main.object(forInfoDictionaryKey: "BrandAccent") as? String ?? "")
    }

    var body: some View {
        if let newest {
            VStack(alignment: .leading, spacing: 4) {
                if family != .accessoryRectangular {
                    Label(newest.topic.isEmpty ? (newest.space ?? "Board") : newest.topic, systemImage: "text.bubble.fill")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(accent)
                        .lineLimit(1)
                        .padding(.bottom, 2)
                }
                Text(newest.text)
                    .font(family == .systemSmall ? .subheadline : .body)
                    .lineLimit(family == .systemMedium ? 3 : 4)
                Spacer(minLength: 0)
                HStack(spacing: 4) {
                    Text(displayName(newest.name)).lineLimit(1)
                    Text("·")
                    // A clock time stays true until the next post reloads the widget; a relative one would tick.
                    Text(newest.time.formatted(Calendar.current.isDateInToday(newest.time) ? .dateTime.hour().minute() : .dateTime.day().month()))
                        .fixedSize()
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        } else {
            VStack(spacing: 6) {
                Image(systemName: "text.bubble")
                    .font(.title2)
                    .foregroundStyle(accent)
                Text("Open \(appName) to see your board.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
        }
    }
}
