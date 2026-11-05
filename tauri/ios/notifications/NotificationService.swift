import Foundation
import Security
import UserNotifications

/// The notification extension inside the Tauri app (spike): it opens each push's note (`n`) with this device's key, read
/// from the Keychain group it shares with the app (where keyring keeps it: service "The One App (Tauri spike)", account
/// "device <address>", base64), and shows what it says, as apple/Notifications does. It also leaves what it did in their
/// App Group, so the spike can see it ran.
final class NotificationService: UNNotificationServiceExtension {
    static let group = "group.app.one.tauri"

    override func didReceive(_ request: UNNotificationRequest, withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void) {
        guard let content = request.content.mutableCopy() as? UNMutableNotificationContent else { return contentHandler(request.content) }
        let shared = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: Self.group)
        let fromApp = shared.flatMap { try? String(contentsOf: $0.appendingPathComponent("from-app.txt"), encoding: .utf8) }
        var report: [String: Any] = ["group": shared?.path ?? "none", "fromApp": fromApp ?? "none"]
        if let pushed = content.userInfo["n"] as? String, let secret = Self.deviceKey(), secret.count == 32 {
            let out = secret.withUnsafeBytes { key in pushed.withCString { one_open_note(key.bindMemory(to: UInt8.self).baseAddress, $0) } }
            let json = out.map { String(cString: $0) } ?? "{}"
            one_free(out)
            let note = (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any] ?? [:]
            if let title = note["title"] as? String {
                content.title = title
                content.body = note["body"] as? String ?? ""
                report["opened"] = note
            } else {
                content.title = "New notification"
                content.body = ""
                report["error"] = note["error"] ?? "couldn't open it"
            }
        } else {
            report["error"] = "no note, or no device key in the Keychain"
        }
        if let shared, let data = try? JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys]) {
            try? data.write(to: shared.appendingPathComponent("opened.json"))
        }
        contentHandler(content)
    }

    static func deviceKey() -> Data? {
        let query: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: "The One App (Tauri spike)",
                                    kSecReturnAttributes as String: true, kSecReturnData as String: true, kSecMatchLimit as String: kSecMatchLimitAll]
        var out: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &out) == errSecSuccess, let items = out as? [[String: Any]] else { return nil }
        for item in items where (item[kSecAttrAccount as String] as? String)?.hasPrefix("device ") == true {
            if let text = (item[kSecValueData as String] as? Data).flatMap({ String(data: $0, encoding: .utf8) }) { return Data(base64Encoded: text) }
        }
        return nil
    }
}
