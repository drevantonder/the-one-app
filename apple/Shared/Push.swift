import Foundation
import UserNotifications
#if os(iOS)
import UIKit
#else
import AppKit
#endif

/// Pushes, in a build with them (`Push` in its Info.plist: apple/project.sh adds it, with the entitlement, once the
/// App ID has push): this device asks to notify, registers its APNs token with the network's push Worker each launch
/// (a token can change), and acts on what's tapped. Its notification extension opens and shows each note (`Notes`).
@MainActor
enum Push {
    static let built = Bundle.main.object(forInfoDictionaryKey: "Push") as? Bool == true
    /// Whether the push Worker took this launch's token.
    private static var registered = false

    /// Registers for this device's token, once it's let in, as the push Worker takes only the network's devices; and
    /// asks to notify, the first time. A token comes whatever the person answers, for pushes that only wake the app.
    static func setUp() async {
        guard built, !registered else { return }
        let center = UNUserNotificationCenter.current()
        center.setNotificationCategories(Notes.categories)
        #if os(iOS)
        UIApplication.shared.registerForRemoteNotifications()
        #else
        NSApplication.shared.registerForRemoteNotifications()
        #endif
        if await center.notificationSettings().authorizationStatus == .notDetermined {
            _ = try? await center.requestAuthorization(options: [.alert, .badge, .sound])
        }
    }

    /// Registers the token Apple gave this device with the push Worker.
    static func register(_ token: Data) async {
        let hex = token.map { String(format: "%02x", $0) }.joined()
        do {
            try await DeviceClient.shared.registerPush(token: hex, env: environment)
            registered = true
        } catch {
            print("push: can't register: \(error.localizedDescription)")
        }
    }

    /// What a tapped notification opens (`<cli>://open?app=…`), after acting on Approve or Not Me, if it was one of
    /// those.
    nonisolated static func answer(_ response: UNNotificationResponse) async -> URL? {
        let info = response.notification.request.content.userInfo
        if let code = info["join"] as? String, [Notes.approve, Notes.notMe].contains(response.actionIdentifier) {
            await join(code, approve: response.actionIdentifier == Notes.approve)
            return nil
        }
        guard let link = info["link"] as? [String: Any], let app = link["app"] as? String else { return nil }
        // Settings' own screens (Settings › Devices, where a device that joined is) are this app's, not an app's.
        if app == "settings" {
            var url = URLComponents()
            url.scheme = Auth.cli
            url.host = "settings"
            url.path = "/\(link["screen"] as? String ?? "")"
            return url.url
        }
        let params = (link["params"] as? [String: Any]).flatMap { try? JSONSerialization.data(withJSONObject: $0) }.map { String(decoding: $0, as: UTF8.self) } ?? "{}"
        var url = URLComponents()
        url.scheme = Auth.cli
        url.host = "open"
        url.queryItems = [.init(name: "app", value: app), .init(name: "screen", value: link["screen"] as? String ?? "main"), .init(name: "params", value: params)]
        return url.url
    }

    /// Lets in the device asking to join with `code`, or turns it down, as this device: the person's node checks it's
    /// theirs. A failure shows as a notification of its own, since nothing else is on screen.
    nonisolated private static func join(_ code: String, approve: Bool) async {
        let body = approve ? ["code": code, "confirm": true] as [String: Any] : ["code": code]
        do {
            let data = try JSONSerialization.data(withJSONObject: body)
            _ = try await DeviceNode().send("POST", approve ? "v1/devices/approve" : "v1/devices/refuse", body: data, timeout: 30)
        } catch {
            let content = UNMutableNotificationContent()
            content.title = approve ? String(localized: "Couldn’t approve that device") : String(localized: "Couldn’t turn that device down")
            content.body = error.localizedDescription
            try? await UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: "join \(code)", content: content, trigger: nil))
        }
    }

    /// Which APNs service this build's pushes come through: what its provisioning profile's `aps-environment` says,
    /// since a Release build installed from Xcode is still signed for development.
    nonisolated static let environment: String = {
        #if os(iOS)
        let profile = Bundle.main.url(forResource: "embedded", withExtension: "mobileprovision")
        let key = "<key>aps-environment</key>"
        #else
        let profile = Optional(Bundle.main.bundleURL.appending(path: "Contents/embedded.provisionprofile"))
        let key = "<key>com.apple.developer.aps-environment</key>"
        #endif
        if let profile, let text = try? String(decoding: Data(contentsOf: profile), as: UTF8.self),
           let at = text.range(of: key), let value = text[at.upperBound...].range(of: "<string>"),
           let end = text[value.upperBound...].range(of: "</string>") {
            return text[value.upperBound..<end.lowerBound] == "development" ? "sandbox" : "production"
        }
        #if DEBUG
        return "sandbox"
        #else
        // No profile: the App Store's.
        return "production"
        #endif
    }()
}
