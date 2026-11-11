import UserNotifications

/// The app's notification extension: it opens each notification's note with this device's key and shows what it says (`Notes`),
/// in the few megabytes the system gives it, so it only opens: no node, no connection, at once.
final class NotificationService: UNNotificationServiceExtension {
    override func didReceive(_ request: UNNotificationRequest, withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void) {
        guard let content = request.content.mutableCopy() as? UNMutableNotificationContent else { return contentHandler(request.content) }
        Notes.show(content)
        contentHandler(content)
    }
}
