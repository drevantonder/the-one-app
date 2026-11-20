import AppIntents
import CoreSpotlight
import UIKit

/// An app in the One App with a screen of its own, as the app last listed them (`shortcuts` in src/shortcuts.rs): in
/// `shortcuts.json` in its Application Support folder, each its entrypoint (`<app>/<page>`), name and icon, a PNG.
@available(iOS 17.0, *)
struct InstalledApp: AppEntity {
    static let typeDisplayRepresentation: TypeDisplayRepresentation = "App"
    static let defaultQuery = InstalledApps()

    let id: String
    let name: String
    let icon: Data

    var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(title: "\(name)", image: .init(data: icon))
    }

    static func listed() -> [InstalledApp] {
        let folder = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        guard let file = try? Data(contentsOf: folder.appendingPathComponent("shortcuts.json")) else { return [] }
        return (try? JSONDecoder().decode([Listed].self, from: file))?.map { InstalledApp(id: $0.id, name: $0.name, icon: $0.icon) } ?? []
    }

    private struct Listed: Decodable {
        let id: String
        let name: String
        let icon: Data
    }
}

/// Each one is in Spotlight under its name and icon, and opens with `OpenApp`.
@available(iOS 18.0, *)
extension InstalledApp: IndexedEntity {
    var attributeSet: CSSearchableItemAttributeSet {
        let attributes = CSSearchableItemAttributeSet()
        attributes.displayName = name
        attributes.thumbnailData = icon
        return attributes
    }
}

@available(iOS 17.0, *)
struct InstalledApps: EnumerableEntityQuery, EntityStringQuery {
    func allEntities() async throws -> [InstalledApp] { InstalledApp.listed() }

    func entities(for identifiers: [String]) async throws -> [InstalledApp] {
        InstalledApp.listed().filter { identifiers.contains($0.id) }
    }

    func entities(matching string: String) async throws -> [InstalledApp] {
        InstalledApp.listed().filter { $0.name.localizedCaseInsensitiveContains(string) }
    }

    func suggestedEntities() async throws -> [InstalledApp] { InstalledApp.listed() }
}

/// Opens an app in the One App, by its `one:` link, which the app opens as any (`links` in src/lib.rs).
@available(iOS 17.0, *)
struct OpenApp: OpenIntent {
    static let title: LocalizedStringResource = "Open App"
    static let description = IntentDescription("Opens an app in the One App.")

    @Parameter(title: "App") var target: InstalledApp

    @MainActor
    func perform() async throws -> some IntentResult {
        if let link = URL(string: "one:\(target.id)") {
            await UIApplication.shared.open(link)
        }
        return .result()
    }
}

/// Tells Device Activity whether this iPhone is on its charger, as a Shortcuts automation runs it when the phone is
/// plugged in or unplugged, without opening the app: the app's Rust tells it (`charger` in src/lib.rs), once its client
/// has opened.
@available(iOS 17.0, *)
struct OnCharger: AppIntent {
    static let title: LocalizedStringResource = "On Charger"
    // Apple refuses an App Intent whose title or description names a device of theirs ("iPhone", ITMS-90626).
    static let description = IntentDescription("Tells your network whether this device is on its charger.")
    static let openAppWhenRun = false

    @Parameter(title: "On Charger") var on: Bool

    static var parameterSummary: some ParameterSummary {
        Summary("On charger: \(\.$on)")
    }

    func perform() async throws -> some IntentResult {
        guard await OneChargerReported.report(charging: on) else {
            throw OnChargerFailed()
        }
        return .result()
    }
}

@available(iOS 17.0, *)
struct OnChargerFailed: Error, CustomLocalizedStringResourceConvertible {
    var localizedStringResource: LocalizedStringResource { "Your network wasn't told. Is the One App signed in?" }
}

/// Where the app's Rust says whether Device Activity was told (`charger` in src/lib.rs), for each `OnCharger` it was
/// given, by the id it was given with.
@objc(OneChargerReported)
final class OneChargerReported: NSObject {
    private static let waiting = NSLock()
    nonisolated(unsafe) private static var reports: [Int: CheckedContinuation<Bool, Never>] = [:]
    nonisolated(unsafe) private static var last = 0

    static func report(charging: Bool) async -> Bool {
        guard let charger: AnyClass = NSClassFromString("OneCharger") else { return false }
        return await withCheckedContinuation { reported in
            waiting.lock()
            last += 1
            let id = last
            reports[id] = reported
            waiting.unlock()
            let said = try? JSONSerialization.data(withJSONObject: ["id": id, "charging": charging])
            let text = said.flatMap { String(data: $0, encoding: .utf8) } ?? "{}"
            _ = (charger as AnyObject).perform(NSSelectorFromString("report:"), with: text as NSString)
        }
    }

    @objc static func reported(_ id: Int, ok: Bool) {
        waiting.lock()
        let reported = reports.removeValue(forKey: id)
        waiting.unlock()
        reported?.resume(returning: ok)
    }
}

/// The app's one App Shortcut, which iOS offers in Shortcuts, Spotlight, Siri and the Action button with no setting
/// up: Open [app] in the One App, for each app listed. iOS takes ten an app at most, so it's one, with the app as its
/// parameter.
@available(iOS 17.0, *)
struct Shortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(
            intent: OpenApp(),
            phrases: ["Open \(\.$target) in \(.applicationName)", "Open \(\.$target) with \(.applicationName)"],
            shortTitle: "Open App",
            systemImageName: "square.grid.2x2"
        )
    }
}

/// What the app's Rust says as the apps it lists change (`shortcuts` in src/shortcuts.rs): the App Shortcut offers them,
/// and Spotlight has them and no others.
@objc(OneShortcuts)
final class OneShortcuts: NSObject {
    @objc static func changed() {
        guard #available(iOS 17.0, *) else { return }
        Shortcuts.updateAppShortcutParameters()
        guard #available(iOS 18.0, *) else { return }
        Task {
            let index = CSSearchableIndex.default()
            do {
                try await index.deleteAppEntities(ofType: InstalledApp.self)
                try await index.indexAppEntities(InstalledApp.listed())
            } catch {
                NSLog("Spotlight didn't take this app's apps: \(error)")
            }
        }
    }
}
