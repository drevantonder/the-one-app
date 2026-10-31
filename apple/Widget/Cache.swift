import Foundation

/// The pages' widgets, as the app last saw them, which it leaves in its app group for the home-screen widget.
struct WidgetCache: Codable {
    struct Item: Codable {
        var title: String
        var icon: String
        var root: WidgetNode
    }
    var install: String?
    var widgets: [Item]

    private static var file: URL? {
        guard let group = Bundle.main.object(forInfoDictionaryKey: "AppGroup") as? String else { return nil }
        return FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group)?.appending(path: "widgets.json")
    }

    static func load() -> WidgetCache? {
        file.flatMap { try? Data(contentsOf: $0) }.flatMap { try? JSONDecoder().decode(WidgetCache.self, from: $0) }
    }

    func save() {
        guard let file = Self.file, let data = try? JSONEncoder().encode(self) else { return }
        try? data.write(to: file, options: .atomic)
    }
}
