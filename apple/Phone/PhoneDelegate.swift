import SwiftUI
import UserNotifications

/// The phone's own part of pushes (`Push`): its APNs token, a notification that arrives while it's open, and a tapped
/// one's screen, or its Approve or Not Me.
@MainActor
final class PhoneDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
    weak var phone: Phone? {
        didSet { if let link = tapped { tapped = nil; phone?.open(link) } }
    }
    /// A notification tapped before the phone was ready for it.
    private var tapped: URL?

    func application(_ application: UIApplication, didFinishLaunchingWithOptions options: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        UNUserNotificationCenter.current().delegate = self
        // The person's Appearance from a window's first frame, not a flash of the other one before SwiftUI's
        // preferredColorScheme reaches it.
        NotificationCenter.default.addObserver(forName: UIWindow.didBecomeVisibleNotification, object: nil, queue: nil) { note in
            let window = note.object as? UIWindow
            MainActor.assumeIsolated {
                let chosen = UserDefaults.standard.string(forKey: "appearance").flatMap(Appearance.init(rawValue:)) ?? .automatic
                window?.overrideUserInterfaceStyle = switch chosen {
                case .automatic: .unspecified
                case .light: .light
                case .dark: .dark
                }
            }
        }
        return true
    }

    func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken token: Data) {
        Task { await Push.register(token) }
    }

    func application(_ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: Error) {
        print("push: no token: \(error.localizedDescription)")
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
        [.banner, .list, .sound]
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        guard let link = await Push.answer(response) else { return }
        await MainActor.run {
            if let phone { phone.open(link) } else { tapped = link }
        }
    }
}

extension Phone {
    /// Every app's badge, and the requests, on the app's icon.
    func showBadge(_ count: Int) {
        UNUserNotificationCenter.current().setBadgeCount(count)
    }
}
