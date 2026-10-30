import Foundation

/// The board's newest post, which the app leaves in its app group for the widget.
struct Newest: Codable {
    var name: String
    var text: String
    var topic: String
    var time: Date
    var space: String?

    private static var file: URL? {
        guard let group = Bundle.main.object(forInfoDictionaryKey: "AppGroup") as? String else { return nil }
        return FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group)?.appending(path: "newest.json")
    }

    static func load() -> Newest? {
        file.flatMap { try? Data(contentsOf: $0) }.flatMap { try? JSONDecoder().decode(Newest.self, from: $0) }
    }

    func save() {
        guard let file = Self.file, let data = try? JSONEncoder().encode(self) else { return }
        try? data.write(to: file, options: .atomic)
    }
}
