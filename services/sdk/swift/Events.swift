@preconcurrency import EventKit
import Foundation

/// Reminders' and Calendar's EventKit store, one per helper. Each call fetches its own objects and is done with them
/// before it answers, so no object crosses calls.
public final class Events: @unchecked Sendable {
    public static let shared = Events()
    public let store = EKEventStore()

    public func status(_ type: EKEntityType) -> Access {
        switch EKEventStore.authorizationStatus(for: type) {
        case .fullAccess: .granted
        case .notDetermined: .unasked
        default: .denied
        }
    }

    /// What the helper needs before it answers: full access to `type`, asked for once.
    public func needs(_ type: EKEntityType) -> String? {
        let reminders = type == .reminder
        return Helper.needs(reminders ? "your reminders" : "full access to your calendars", status: status(type), ask: {
            if reminders { store.requestFullAccessToReminders { _, _ in } } else { store.requestFullAccessToEvents { _, _ in } }
        }, settings: reminders ? "Reminders" : "Calendars")
    }

    /// Fails a call once macOS took the access back.
    public func access(_ type: EKEntityType) throws {
        try permission(status(type), denied: type == .reminder
            ? "macOS doesn't let \(thisApp) use Reminders. Allow it in System Settings → Privacy & Security → Reminders: turn on \(thisApp)."
            : "macOS doesn't let \(thisApp) see your calendars. Allow it in System Settings → Privacy & Security → Calendars: set \(thisApp) to Full Access.")
    }

    public func calendar(_ type: EKEntityType, _ name: String?) throws -> EKCalendar? {
        guard let name else { return nil }
        let all = store.calendars(for: type)
        if let c = all.first(where: { $0.calendarIdentifier == name }) ?? all.first(where: { $0.title.localizedCaseInsensitiveCompare(name) == .orderedSame }) {
            return c
        }
        throw Refusal("no \(type == .reminder ? "list" : "calendar") called \(name); there are \(all.map(\.title).joined(separator: ", "))")
    }

    public func reminders(_ predicate: NSPredicate) async -> [EKReminder] {
        struct Found: @unchecked Sendable { var reminders: [EKReminder] }
        return await withCheckedContinuation { (done: CheckedContinuation<Found, Never>) in
            store.fetchReminders(matching: predicate) { done.resume(returning: Found(reminders: $0 ?? [])) }
        }.reminders
    }
}
