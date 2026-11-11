import CryptoKit
import Foundation
import SwiftUI

/// Where the app leaves what widgets draw from: the iPhone's App Group; on a Mac, the folder the app says.
enum WidgetFiles {
    #if os(macOS)
    nonisolated(unsafe) static var dir: URL?
    #else
    static var dir: URL? {
        guard let group = Bundle.main.object(forInfoDictionaryKey: "AppGroup") as? String else { return nil }
        return FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group)
    }
    #endif
}

/// Every page's widgets, as the app last saw them, which it leaves for the platform's own (`WidgetFiles`).
struct WidgetCache: Codable {
    /// One of the user's devices, as a Mac's menu bar lists them.
    struct Device: Codable, Equatable {
        var key: String
        var name: String
        var online: Bool
    }

    struct Item: Codable, Identifiable, Equatable {
        /// What a home-screen widget's choice is found again by.
        var id: String
        /// `<app>.<widget>`, the same on every page.
        var kind: String?
        var app: String?
        var widget: String?
        /// The page it's on, and that page's name.
        var scope: String?
        var page: String?
        var title: String
        var icon: String
        /// Its app's picture, a brand's logo or app icon, as a data URL, and `icon` when it's a finished app icon.
        var image: String?
        var imageKind: String?
        /// Whether its app has a first screen to open.
        var opens: Bool?
        var error: String?
        /// Its most prominent text, for a one-line place.
        var headline: String?
        var root: WidgetNode
        /// Its tree at each size it draws.
        var sizes: [String: WidgetNode]?

        func tree(_ size: String) -> WidgetNode { sizes?[size] ?? root }
    }
    var network: String?
    var devices: [Device]?
    var widgets: [Item]
    /// The user signed out, so the home-screen widgets say to sign in.
    var signedOut: Bool?
    /// When the app last had them from your devices.
    var fetched: Date?

    private static var file: URL? { WidgetFiles.dir?.appending(path: "widgets.json") }

    static func load() -> WidgetCache? {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .secondsSince1970
        return file.flatMap { try? Data(contentsOf: $0) }.flatMap { try? decoder.decode(WidgetCache.self, from: $0) }
    }

    func save() {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .secondsSince1970
        guard let file = Self.file, let data = try? encoder.encode(self) else { return }
        try? data.write(to: file, options: .atomic)
    }

    /// Signed out: none of the user's widgets or their images are left for the home screen's.
    static func forget() {
        WidgetCache(widgets: [], signedOut: true).save()
        WidgetImages.forget()
    }
}

/// The images widgets show, saved by the app where its home-screen widgets can read them: they can't download.
enum WidgetImages {
    private static var dir: URL? {
        WidgetFiles.dir?.appending(path: "images")
    }

    private static func file(_ url: String) -> URL? {
        let name = SHA256.hash(data: Data(url.utf8)).prefix(12).map { String(format: "%02x", $0) }.joined()
        guard let dir else { return nil }
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir.appending(path: name)
    }

    static func forget() {
        if let dir { try? FileManager.default.removeItem(at: dir) }
    }

    static func load(_ url: String) -> Image? {
        file(url).flatMap { PlatformImage(contentsOfFile: $0.path(percentEncoded: false)) }.map { Image(platform: $0) }
    }

    /// Downloads the ones not saved yet, small enough for a widget to draw.
    static func save(_ urls: [URL]) async {
        for url in Set(urls) where url.scheme == "https" {
            guard let file = file(url.absoluteString), !FileManager.default.fileExists(atPath: file.path(percentEncoded: false)) else { continue }
            guard let (data, _) = try? await URLSession.shared.data(from: url), data.count < 2_000_000 else { continue }
            try? data.write(to: file, options: .atomic)
        }
    }
}
