import Foundation
import Security
import UserNotifications

/// The notification extension inside MyGo's app (spike): it opens each push's note (`n`) with this device's key, read
/// from the Keychain group it shares with the app (where mygo.Secrets keeps it: service "one", account "device"), and
/// shows what it says, as apple/Notifications does. It also reads what the app left in their App Group, and leaves
/// there what it opened, so the spike can see it ran.
final class NotificationService: UNNotificationServiceExtension {
    static let group = "group.app.one.mygo"

    override func didReceive(_ request: UNNotificationRequest, withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void) {
        guard let content = request.content.mutableCopy() as? UNMutableNotificationContent else { return contentHandler(request.content) }
        let shared = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: Self.group)
        let fromApp = shared.flatMap { try? String(contentsOf: $0.appending(path: "from-app.txt"), encoding: .utf8) }
        var report: [String: Any] = ["group": shared?.path ?? "none", "fromApp": fromApp ?? "none"]
        if let pushed = content.userInfo["n"] as? String, let secret = Self.deviceKey() {
            do {
                let note = try openNote(secret: secret, pushed: pushed)
                content.title = note.title
                content.body = note.body
                report["opened"] = ["title": note.title, "body": note.body, "app": note.app, "by": note.by ?? ""]
            } catch {
                content.title = "New notification"
                content.body = ""
                report["error"] = "\(error)"
            }
        } else {
            report["error"] = "no note, or no device key in the Keychain"
        }
        if let shared, let data = try? JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys]) {
            try? data.write(to: shared.appending(path: "opened.json"))
        }
        contentHandler(content)
    }

    static func deviceKey() -> Data? {
        let query: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: "one", kSecAttrAccount as String: "device",
                                    kSecReturnData as String: true, kSecMatchLimit as String: kSecMatchLimitOne]
        var out: CFTypeRef?
        return SecItemCopyMatching(query as CFDictionary, &out) == errSecSuccess ? out as? Data : nil
    }
}
