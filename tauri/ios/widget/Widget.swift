import SwiftUI
import WidgetKit

/// A widget inside the Tauri app (spike): it shows what the app left in their App Group, as apple/Widget draws what the
/// app caches there. A target of the Xcode project Tauri generates, from ios/project.yml.
struct Entry: TimelineEntry {
    let date: Date
    let text: String
}

struct Provider: TimelineProvider {
    func placeholder(in context: Context) -> Entry { Entry(date: .now, text: "…") }
    func getSnapshot(in context: Context, completion: @escaping (Entry) -> Void) { completion(read()) }
    func getTimeline(in context: Context, completion: @escaping (Timeline<Entry>) -> Void) {
        completion(Timeline(entries: [read()], policy: .after(.now.addingTimeInterval(900))))
    }

    func read() -> Entry {
        let shared = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: "group.app.one.tauri")
        let text = shared.flatMap { try? String(contentsOf: $0.appending(path: "from-app.txt"), encoding: .utf8) } ?? "Nothing from the app"
        return Entry(date: .now, text: text)
    }
}

struct SpikeWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "spike", provider: Provider()) { entry in
            Text(entry.text).font(.caption).containerBackground(.background, for: .widget)
        }
        .configurationDisplayName("One (Tauri)")
        .description("What the app left in its App Group.")
    }
}

@main
struct Widgets: WidgetBundle {
    var body: some Widget { SpikeWidget() }
}
