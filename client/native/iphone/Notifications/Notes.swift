import Foundation
import UserNotifications

/// What's sent to this device: each notification carries a note sealed to it (notifications/src/lib.rs), which this extension opens with
/// this device's key and shows as its notification, with the app screen a tap opens (`link`). A note whose action is a
/// device asking to join (`{ "join": "<its code>" }`) shows Approve and Not Me, which the app acts on (its plugin's
/// `Notes`, iphone/Sources/OneNative), where the user's node checks the request is theirs.
enum Notes {
    /// A device asking to join: its notification's category, whose Approve and Not Me the app registers and answers.
    static let join = "join"

    /// Shows the note `content` carries (`n`), opened with one of this device's keys; or, if none opens it, only that one
    /// came.
    static func show(_ content: UNMutableNotificationContent) {
        guard let carried = content.userInfo["n"] as? String,
              let note = DeviceKeys.all().lazy.compactMap({ try? openNote(secret: $0, carried: carried) }).first else {
            // What the notifications Worker wrote is what shows: that one came, and nothing of it.
            content.userInfo = [:]
            return
        }
        content.title = note.title
        content.body = note.body
        var info: [String: Any] = [:]
        if let link = note.link { info["link"] = link }
        if !note.app.isEmpty { content.threadIdentifier = note.app }
        if let code = note.join {
            info["join"] = code
            content.categoryIdentifier = join
        }
        content.userInfo = info
    }
}
