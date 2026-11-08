import Foundation
import UserNotifications

/// What's pushed to this device: each push carries a note sealed to it (src/push.rs), which this extension opens with
/// this device's key and shows as its notification, with the app screen a tap opens (`link`). A note whose action is a
/// device asking to join (`{ "join": "<its code>" }`) shows Approve and Not Me, which the app acts on (its plugin's
/// `Notes`, iphone/Sources/OneNative), where the user's node checks the request is theirs.
enum Notes {
    /// A device asking to join: its notification's category, whose Approve and Not Me the app registers and answers.
    static let join = "join"

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
        if let link = note.link { info["link"] = link }
        if !note.app.isEmpty { content.threadIdentifier = note.app }
        if let code = note.join {
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
