import SwiftUI
import UserNotifications

/// The inbox's pushes: the phone registers its APNs token with the brand's inbox app each launch, shows a push that
/// arrives while it's open, and opens a tapped one's `link` as an app screen.
@MainActor
final class PhoneDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
    weak var phone: Phone? {
        didSet { if let route = tapped { tapped = nil; phone?.store.path.append(route) } }
    }
    /// A push tapped before the phone was ready for it.
    private var tapped: Route?

    func application(_ application: UIApplication, didFinishLaunchingWithOptions options: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        UNUserNotificationCenter.current().delegate = self
        return true
    }

    func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken token: Data) {
        let hex = token.map { String(format: "%02x", $0) }.joined()
        Task { await phone?.register(token: hex) }
    }

    func application(_ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: Error) {
        print("push: can't register: \(error.localizedDescription)")
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
        [.banner, .list, .sound]
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        // `link` is `{ app, screen, params }`, as the inbox app's notify was given it.
        guard let link = response.notification.request.content.userInfo["link"] as? [String: Any], let app = link["app"] as? String else { return }
        let params = (link["params"] as? [String: Any]).flatMap { try? JSONSerialization.data(withJSONObject: $0) }.map { String(decoding: $0, as: UTF8.self) } ?? "{}"
        let route = Route.screen(ScreenRoute(app: app, screen: link["screen"] as? String ?? "main", params: params))
        await MainActor.run {
            if let phone { phone.store.path.append(route) } else { tapped = route }
        }
    }
}

extension Phone {
    /// Asks to notify once the phone is in an install whose inbox it can read, then registers for pushes: each launch,
    /// since a token can change.
    func setUpPush() async {
        // A build has push only once it's on for the brand (apple/project.sh): without the entitlement, no token.
        guard Bundle.main.object(forInfoDictionaryKey: "Push") as? Bool == true else { return }
        guard Self.nodeURL == nil, store.inbox != nil else { return }
        let center = UNUserNotificationCenter.current()
        var status = await center.notificationSettings().authorizationStatus
        if status == .notDetermined {
            _ = try? await center.requestAuthorization(options: [.alert, .badge, .sound])
            status = await center.notificationSettings().authorizationStatus
        }
        guard [.authorized, .provisional, .ephemeral].contains(status) else { return }
        UIApplication.shared.registerForRemoteNotifications()
    }

    /// Tells the inbox app where to push to: APNs' sandbox for a build signed for development, its production service
    /// for one from the App Store or TestFlight.
    func register(token: String) async {
        guard let inbox = store.inbox else { return }
        let input = ["token": token, "platform": "ios", "env": Self.apnsEnvironment, "name": store.state?.machine.name ?? UIDevice.current.name]
        guard let data = try? JSONSerialization.data(withJSONObject: input) else { return }
        do {
            _ = try await store.node.call(app: inbox.name, op: "register", input: data)
        } catch {
            print("push: can't register with \(inbox.name): \(error.localizedDescription)")
        }
    }

    /// The unread count on the app's icon.
    func showBadge(_ count: Int) {
        UNUserNotificationCenter.current().setBadgeCount(count)
    }

    /// Which APNs service this build's pushes come through: what its provisioning profile's `aps-environment` says,
    /// since a Release build installed from Xcode is still signed for development.
    static let apnsEnvironment: String = {
        if let url = Bundle.main.url(forResource: "embedded", withExtension: "mobileprovision"),
           let profile = try? String(decoding: Data(contentsOf: url), as: UTF8.self),
           let key = profile.range(of: "<key>aps-environment</key>"),
           let value = profile[key.upperBound...].range(of: "<string>"),
           let end = profile[value.upperBound...].range(of: "</string>") {
            return profile[value.upperBound..<end.lowerBound] == "production" ? "production" : "sandbox"
        }
        #if DEBUG
        return "sandbox"
        #else
        // No profile: the App Store's.
        return "production"
        #endif
    }()
}
