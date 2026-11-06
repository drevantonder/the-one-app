import AppIntents
import SwiftUI
import WidgetKit

@main
struct Widgets: WidgetBundle {
    var body: some Widget { PagesWidget() }
}

/// A widget from your pages, the one you choose (the first, until you do), drawn as the app draws it.
struct PagesWidget: Widget {
    var body: some WidgetConfiguration {
        AppIntentConfiguration(kind: "widget", intent: ChooseWidget.self, provider: Provider()) { entry in
            EntryView(entry: entry)
                .containerBackground(Kit.bg, for: .widget)
        }
        .configurationDisplayName(appName)
        .description("A widget from one of your pages.")
        .supportedFamilies([.systemSmall, .systemMedium, .systemLarge, .systemExtraLarge, .accessoryRectangular, .accessoryInline])
    }
}

/// Which widget a home-screen, desktop or lock-screen widget shows.
struct ChooseWidget: WidgetConfigurationIntent {
    static let title: LocalizedStringResource = "Choose a Widget"
    static let description = IntentDescription("Pick which of your pages’ widgets this shows.")

    @Parameter(title: "Widget") var widget: WidgetChoice?
}

/// A widget on one of the pages, found again by its id alone, so a choice outlives a slow or empty cache.
struct WidgetChoice: AppEntity {
    static let typeDisplayRepresentation: TypeDisplayRepresentation = "Widget"
    static let defaultQuery = WidgetChoices()

    var id: String
    var title: String
    var page: String

    var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(title: "\(title)", subtitle: page.isEmpty ? nil : "\(page)")
    }

    init(_ item: WidgetCache.Item) {
        id = item.id
        title = item.title
        page = item.page ?? ""
    }

    init(id: String) {
        self.id = id
        title = String(id.split(separator: ".").last ?? "")
        page = ""
    }
}

struct WidgetChoices: EntityQuery {
    func entities(for identifiers: [String]) async throws -> [WidgetChoice] {
        let items = WidgetCache.load()?.widgets ?? []
        return identifiers.map { id in items.first { $0.id == id }.map(WidgetChoice.init) ?? WidgetChoice(id: id) }
    }

    func suggestedEntities() async throws -> [WidgetChoice] {
        (WidgetCache.load()?.widgets ?? []).map(WidgetChoice.init)
    }
}

struct Entry: TimelineEntry {
    var date = Date.now
    var widget: WidgetCache.Item?
    /// When the app last had it from your devices.
    var fetched: Date?
    /// The widget chosen, if it's gone: its title.
    var missing: String?
    var signedOut = false
}

/// The app reloads the timelines whenever its widgets change; between reloads, times count on from each entry, and a
/// widget that asks to be drawn again at `next` gets a reload then.
struct Provider: AppIntentTimelineProvider {
    func placeholder(in context: Context) -> Entry {
        Entry(widget: .init(title: appName, icon: "app", root: WidgetNode(t: "col", c: [
            WidgetNode(t: "text", text: "MacBook Air", size: "sm", weight: "medium"),
            WidgetNode(t: "text", text: "Deployed the new build to every device.", lines: 3),
        ])))
    }

    func snapshot(for configuration: ChooseWidget, in context: Context) async -> Entry {
        context.isPreview && WidgetCache.load() == nil ? placeholder(in: context) : entry(for: configuration)
    }

    func timeline(for configuration: ChooseWidget, in context: Context) async -> Timeline<Entry> {
        let first = entry(for: configuration)
        let entries = (0..<36).map { i in
            var entry = first
            entry.date = .now.addingTimeInterval(Double(i) * 300)
            return entry
        }
        if let next = first.widget?.root.redraw() { return Timeline(entries: entries.filter { $0.date < next }, policy: .after(next)) }
        return Timeline(entries: entries, policy: .atEnd)
    }

    private func entry(for configuration: ChooseWidget) -> Entry {
        let cache = WidgetCache.load()
        if cache?.signedOut == true { return Entry(signedOut: true) }
        let items = cache?.widgets ?? []
        guard let choice = configuration.widget else { return Entry(widget: items.first, fetched: cache?.fetched) }
        if let item = items.first(where: { $0.id == choice.id }) { return Entry(widget: item, fetched: cache?.fetched) }
        return items.isEmpty ? Entry() : Entry(missing: choice.title.isEmpty ? String(localized: "This widget") : choice.title)
    }
}

struct EntryView: View {
    var entry: Entry
    @Environment(\.widgetFamily) private var family

    /// The page size drawn at this family: the same tree the app's pages show at that size.
    private var size: String {
        switch family {
        case .systemSmall: "small"
        case .systemLarge: "large"
        case .systemExtraLarge: "xl"
        default: "medium"
        }
    }

    var body: some View {
        if let widget = entry.widget {
            let host = WidgetHost(app: widget.app ?? "", scope: widget.scope)
            Group {
                if family == .accessoryInline {
                    Label { Text(widget.headline ?? widget.title) } icon: { Glyph(name: widget.icon) }
                } else if widget.error != nil {
                    Broken(title: widget.title, text: "This widget stopped working. Open \(appName) to see why.")
                } else {
                    let tree = widget.tree(size)
                    // Over an hour old, it's dimmed and says when it's from.
                    let old = entry.fetched.flatMap { entry.date.timeIntervalSince($0) > 3600 ? $0 : nil }
                    VStack(alignment: .leading, spacing: 6) {
                        if family != .accessoryRectangular, tree.header != false {
                            WidgetLabel(title: widget.title, icon: widget.icon, image: widget.image, imageKind: widget.imageKind)
                        }
                        WidgetTree(node: tree, now: entry.date)
                            .font(.footnote)
                            .environment(\.widgetHost, host)
                            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                            .opacity(old == nil ? 1 : 0.5)
                        if let old, family != .accessoryRectangular {
                            Text("Updated \(RelativeDateTimeFormatter().localizedString(for: old, relativeTo: entry.date))")
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                        }
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                }
            }
            .widgetURL((widget.tree(size).link ?? .screen("main", params: nil)).url(app: host.app, scope: host.scope))
        } else if let missing = entry.missing {
            Broken(title: appName, text: "\(missing) isn’t on your pages any more. Touch and hold to choose another.")
        } else if family == .accessoryInline {
            Text(appName)
        } else {
            VStack(spacing: 8) {
                Image("Logo")
                    .resizable()
                    .frame(width: 44, height: 44)
                Text(entry.signedOut ? "Sign in to \(appName) to see your pages." : "Open \(appName) to see your pages.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
        }
    }
}

/// A widget that can't be drawn, and what to do about it.
private struct Broken: View {
    var title: String
    var text: String

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Label(title, systemImage: "exclamationmark.triangle.fill")
                .font(.caption.weight(.semibold))
                .foregroundStyle(Kit.warn)
                .lineLimit(1)
            Text(text)
                .font(.footnote)
                .foregroundStyle(.secondary)
            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }
}
