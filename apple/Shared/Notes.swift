import Foundation
import UserNotifications

/// What's pushed to this device: each push carries a note sealed to it (src/push.rs), which the app's notification
/// extension opens with this device's key and shows as its notification, with the app screen a tap opens (`link`). A
/// note whose action is a device asking to join (`{ "join": "<its code>" }`) shows Approve and Not Me, which the app
/// acts on (`PushActions`), where the person's node checks the request is theirs.
enum Notes {
    /// A device asking to join: its notification's category, and its two actions.
    static let join = "join"
    static let approve = "approve"
    static let notMe = "not-me"

    /// The categories the app registers, for its notifications' actions.
    static var categories: Set<UNNotificationCategory> {
        let actions = [
            UNNotificationAction(identifier: approve, title: String(localized: "Approve"), options: [.authenticationRequired]),
            UNNotificationAction(identifier: notMe, title: String(localized: "Not Me"), options: [.authenticationRequired, .destructive]),
        ]
        return [UNNotificationCategory(identifier: join, actions: actions, intentIdentifiers: [])]
    }

    /// Shows the note `content` carries (`n`), opened with one of this device's keys; or, if none opens it, only that one
    /// came.
    static func show(_ content: UNMutableNotificationContent) {
        guard let pushed = content.userInfo["n"] as? String,
              let note = DeviceKeys.all().lazy.compactMap({ try? openNote(secret: $0, pushed: pushed) }).first else {
            showNothing(content)
            return
        }
        content.title = note.title
        content.body = note.body
        if let badge = note.badge { content.badge = NSNumber(value: badge) }
        var info: [String: Any] = [:]
        if !note.app.isEmpty {
            let params = (try? JSONSerialization.jsonObject(with: Data(note.params.utf8))) ?? [String: Any]()
            info["link"] = ["app": note.app, "screen": note.screen.isEmpty ? "main" : note.screen, "params": params]
            content.threadIdentifier = note.app
        }
        let action = note.action.flatMap { try? JSONSerialization.jsonObject(with: Data($0.utf8)) as? [String: Any] }
        if let code = action?["join"] as? String, !code.isEmpty {
            info["join"] = code
            content.categoryIdentifier = join
        }
        content.userInfo = info
    }

    /// A push that didn't open here: that one came, and nothing of it.
    static func showNothing(_ content: UNMutableNotificationContent) {
        content.title = String(localized: "New notification")
        content.body = ""
        content.userInfo = [:]
    }
}
