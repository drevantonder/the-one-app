@preconcurrency import EventKit
import Foundation
import Helper

struct RemindersApp: Service {
    let app = "reminders"
    let localApp: String? = "com.apple.reminders"

    func needs() async -> String? { Events.shared.needs(.reminder) }

    func run(_ function: String, _ input: Input) async throws -> Any {
        let ek = Events.shared, store = ek.store
        try ek.access(.reminder)
        switch function {
        case "lists":
            let preferred = store.defaultCalendarForNewReminders()?.calendarIdentifier
            return store.calendars(for: .reminder).map { c in
                object(["name": c.title, "id": c.calendarIdentifier, "account": c.source?.title, "default": c.calendarIdentifier == preferred ? true : nil])
            }
        case "list":
            let lists = try ek.calendar(.reminder, input.string("list")).map { [$0] }
            let until = try input.date("until").map { $0.day ? Calendar.current.date(byAdding: .day, value: 1, to: $0.date)! : $0.date }
            let completed = input.bool("completed") ?? false
            var found = await ek.reminders(completed ? store.predicateForReminders(in: lists)
                : store.predicateForIncompleteReminders(withDueDateStarting: nil, ending: until, calendars: lists))
            if completed, let until { found = found.filter { due($0).map { $0 < until } ?? false } }
            if let q = input.string("query") {
                found = found.filter { ($0.title ?? "").localizedCaseInsensitiveContains(q) || ($0.notes ?? "").localizedCaseInsensitiveContains(q) }
            }
            found.sort { (due($0) ?? .distantFuture, $0.title ?? "") < (due($1) ?? .distantFuture, $1.title ?? "") }
            return found.prefix(input.int("limit", or: 100)).map(json)
        case "create":
            let r = EKReminder(eventStore: store)
            r.title = try input.text("title")
            guard let list = try ek.calendar(.reminder, input.string("list")) ?? store.defaultCalendarForNewReminders() else {
                throw Refusal("there's no default list; give one")
            }
            r.calendar = list
            r.notes = input.string("notes")
            try edit(r, input)
            try store.save(r, commit: true)
            return json(r)
        case "update":
            let r = try reminder(input)
            if let title = input.string("title") { r.title = title }
            if let list = try ek.calendar(.reminder, input.string("list")) { r.calendar = list }
            if input.has("notes") { r.notes = input.string("notes") }
            if let completed = input.bool("completed") { r.isCompleted = completed }
            try edit(r, input)
            try store.save(r, commit: true)
            return json(r)
        case "delete":
            let r = try reminder(input)
            try store.remove(r, commit: true)
            return "Deleted \(r.title ?? "the reminder")."
        default:
            throw Refusal("reminders has no function \(function)")
        }
    }

    private func reminder(_ input: Input) throws -> EKReminder {
        let id = try input.text("id")
        guard let r = Events.shared.store.calendarItem(withIdentifier: id) as? EKReminder else { throw Refusal("no reminder with id \(id); reminders_list lists them") }
        return r
    }

    /// Sets the due date and priority a call gives. A due time also alerts then, as it does when set in Reminders.
    private func edit(_ r: EKReminder, _ input: Input) throws {
        if let p = input.string("priority") {
            guard let value = ["none": 0, "high": 1, "medium": 5, "low": 9][p.lowercased()] ?? Int(p) else { throw Refusal("priority is high, medium, low or none") }
            r.priority = value
        }
        guard input.has("due") else { return }
        r.alarms?.forEach(r.removeAlarm)
        if ["none", "null", ""].contains(input.string("due")?.lowercased() ?? "") {
            r.dueDateComponents = nil
            return
        }
        guard let when = try input.date("due") else { return }
        r.dueDateComponents = Calendar.current.dateComponents(when.day ? [.year, .month, .day] : [.year, .month, .day, .hour, .minute], from: when.date)
        if !when.day { r.addAlarm(EKAlarm(absoluteDate: when.date)) }
    }

    private func due(_ r: EKReminder) -> Date? {
        r.dueDateComponents.flatMap { Calendar.current.date(from: $0) }
    }

    private func json(_ r: EKReminder) -> [String: Any] {
        let timed = r.dueDateComponents?.hour != nil
        let priority = switch r.priority { case 1...4: "high"; case 5: "medium"; case 6...9: "low"; default: String?.none }
        return object([
            "id": r.calendarItemIdentifier, "title": r.title ?? "", "list": r.calendar?.title,
            "due": due(r).map { timed ? iso($0) : isoDay($0) }, "priority": priority, "notes": r.notes?.nonEmpty,
            "completed": r.isCompleted ? iso(r.completionDate ?? .now) : nil,
        ])
    }
}
