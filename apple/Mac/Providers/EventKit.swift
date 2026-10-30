@preconcurrency import EventKit
import Foundation

/// Reminders and Calendar share one EventKit store. Each call fetches its own objects and is done with them
/// before it answers, so no object crosses calls.
final class Events: @unchecked Sendable {
    static let shared = Events()
    let store = EKEventStore()

    func access(_ type: EKEntityType) async throws {
        let reminders = type == .reminder
        try await permission(reminders ? "Reminders" : "Calendar", status: {
            switch EKEventStore.authorizationStatus(for: type) {
            case .fullAccess: .granted
            case .notDetermined: .unasked
            default: .denied
            }
        }, ask: {
            if reminders { store.requestFullAccessToReminders { _, _ in } } else { store.requestFullAccessToEvents { _, _ in } }
        }, denied: reminders
            ? "macOS doesn't let \(appName) use Reminders. Allow it in System Settings → Privacy & Security → Reminders: turn on \(appName)."
            : "macOS doesn't let \(appName) see your calendars. Allow it in System Settings → Privacy & Security → Calendars: set \(appName) to Full Access.")
    }

    /// The list or calendar with this name or id; nil when none is given.
    func calendar(_ type: EKEntityType, _ name: String?) throws -> EKCalendar? {
        guard let name else { return nil }
        let all = store.calendars(for: type)
        if let c = all.first(where: { $0.calendarIdentifier == name }) ?? all.first(where: { $0.title.localizedCaseInsensitiveCompare(name) == .orderedSame }) {
            return c
        }
        throw Refusal("no \(type == .reminder ? "list" : "calendar") called \(name); there are \(all.map(\.title).joined(separator: ", "))")
    }

    func reminders(_ predicate: NSPredicate) async -> [EKReminder] {
        struct Found: @unchecked Sendable { var reminders: [EKReminder] }
        return await withCheckedContinuation { (done: CheckedContinuation<Found, Never>) in
            store.fetchReminders(matching: predicate) { done.resume(returning: Found(reminders: $0 ?? [])) }
        }.reminders
    }
}

extension String {
    var nonEmpty: String? { isEmpty ? nil : self }
}

struct RemindersApp: Provider {
    let app = "reminders"

    var description: [String: Any] {
        let priority = str("high, medium, low or none")
        return describe(title: "Reminders", icon: "list-checks", instructions:
            "Reminders are the person's Apple Reminders, through their Mac, and sync to their other Apple devices. reminders_lists lists the lists; " +
            "reminders_list lists reminders, soonest due first, leaving out completed ones unless asked. reminders_update also completes one.", ops: [
            op("lists", "The reminder lists."),
            op("list", "Reminders, soonest due first; only those not completed, unless asked.", [
                "list": str("a list's name; omit for every list"),
                "until": str("only reminders due by then: an ISO 8601 date or date-time"),
                "query": str("only reminders whose title or notes contain this"),
                "completed": flag("include completed reminders"),
                "limit": num("at most this many; default 100"),
            ]),
            op("create", "Create a reminder.", [
                "title": str("what to be reminded of"),
                "list": str("a list's name; default is the default list"),
                "due": str("an ISO 8601 date (2026-11-01), or a date-time (2026-11-01T09:00) to be alerted then"),
                "notes": str("more detail"),
                "priority": priority,
            ], required: ["title"]),
            op("update", "Change a reminder, or complete it.", [
                "id": str("the reminder's id, as list gives it"),
                "title": str("a new title"),
                "list": str("move it to this list"),
                "due": str("an ISO 8601 date or date-time, or \"none\" to clear it"),
                "notes": str("new notes"),
                "priority": priority,
                "completed": flag("true completes it; false makes it not done again"),
            ], required: ["id"]),
            op("delete", "Delete a reminder.", ["id": str("the reminder's id, as list gives it")], required: ["id"]),
        ])
    }

    func run(_ op: String, _ input: Input) async throws -> Any {
        let ek = Events.shared, store = ek.store
        try await ek.access(.reminder)
        switch op {
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
            throw Refusal("reminders has no operation \(op)")
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

struct CalendarApp: Provider {
    let app = "calendar"

    var description: [String: Any] {
        let fields: [String: [String: Any]] = [
            "title": str("what it is"),
            "start": str("an ISO 8601 date-time (2026-11-01T09:00), or a date for an all-day event"),
            "end": str("an ISO 8601 date-time, or the last day of an all-day event; default an hour after start, or the same day"),
            "all_day": flag("an all-day event"),
            "calendar": str("a calendar's name; default is the default calendar"),
            "location": str("where"),
            "notes": str("more detail"),
        ]
        let id = str("the event's id, as events gives it; for a repeating event it names one occurrence")
        let future = flag("for a repeating event, this occurrence and every later one, not just this one")
        return describe(title: "Calendar", icon: "calendar", instructions:
            "Calendar is the person's Apple Calendar through their Mac: every account it has, such as iCloud, Google or Exchange. " +
            "calendar_events lists events in a time range, by default the coming week. Times are ISO 8601; without an offset they're in the Mac's time zone.", ops: [
            op("calendars", "The calendars, with whether each can be changed."),
            op("events", "Events in a time range, earliest first.", [
                "from": str("ISO 8601; default the start of today"),
                "to": str("ISO 8601; a date includes that whole day; default a week after from"),
                "calendar": str("a calendar's name; omit for every calendar"),
                "query": str("only events whose title, location or notes contain this"),
                "limit": num("at most this many; default 200"),
            ]),
            op("create", "Create an event.", fields, required: ["title", "start"]),
            op("update", "Change an event: only the fields given.", fields.merging(["id": id, "future": future]) { a, _ in a }, required: ["id"]),
            op("delete", "Delete an event.", ["id": id, "future": future], required: ["id"]),
        ])
    }

    func run(_ op: String, _ input: Input) async throws -> Any {
        let ek = Events.shared, store = ek.store
        try await ek.access(.event)
        switch op {
        case "calendars":
            let preferred = store.defaultCalendarForNewEvents?.calendarIdentifier
            return store.calendars(for: .event).map { c in
                object(["name": c.title, "id": c.calendarIdentifier, "account": c.source?.title, "writable": c.allowsContentModifications,
                        "default": c.calendarIdentifier == preferred ? true : nil])
            }
        case "events":
            let from = try input.date("from")?.date ?? Calendar.current.startOfDay(for: .now)
            let to = try input.date("to").map { $0.day ? Calendar.current.date(byAdding: .day, value: 1, to: $0.date)! : $0.date }
                ?? from.addingTimeInterval(7 * 86400)
            guard to > from else { throw Refusal("to must be after from") }
            let calendars = try ek.calendar(.event, input.string("calendar")).map { [$0] }
            var events = store.events(matching: store.predicateForEvents(withStart: from, end: to, calendars: calendars))
            if let q = input.string("query") {
                events = events.filter { [$0.title, $0.location, $0.notes].contains { $0?.localizedCaseInsensitiveContains(q) ?? false } }
            }
            return events.sorted { $0.startDate < $1.startDate }.prefix(input.int("limit", or: 200)).map(json)
        case "create":
            let e = EKEvent(eventStore: store)
            guard let calendar = try ek.calendar(.event, input.string("calendar")) ?? store.defaultCalendarForNewEvents else {
                throw Refusal("there's no default calendar; give one")
            }
            e.calendar = calendar
            guard input.has("start") else { throw Refusal("give start") }
            try edit(e, input)
            try store.save(e, span: .thisEvent, commit: true)
            return json(e)
        case "update":
            let e = try event(input)
            try edit(e, input)
            try store.save(e, span: input.bool("future") == true ? .futureEvents : .thisEvent, commit: true)
            return json(e)
        case "delete":
            let e = try event(input)
            try store.remove(e, span: input.bool("future") == true ? .futureEvents : .thisEvent, commit: true)
            return "Deleted \(e.title ?? "the event")."
        default:
            throw Refusal("calendar has no operation \(op)")
        }
    }

    /// Sets the fields a call gives. Moving the start keeps the event's length unless the end is given too.
    private func edit(_ e: EKEvent, _ input: Input) throws {
        if let title = input.string("title") { e.title = title }
        if e.title?.isEmpty ?? true { throw Refusal("give title") }
        if let calendar = try Events.shared.calendar(.event, input.string("calendar")) { e.calendar = calendar }
        if e.calendar?.allowsContentModifications == false { throw Refusal("\(e.calendar.title) can't be changed") }
        if input.has("location") { e.location = input.string("location") }
        if input.has("notes") { e.notes = input.string("notes") }
        let length = e.startDate.flatMap { start in e.endDate.map { $0.timeIntervalSince(start) } }
        let start = try input.date("start")
        if let allDay = input.bool("all_day") ?? start.map(\.day) { e.isAllDay = allDay }
        if let start { e.startDate = start.date }
        if let end = try input.date("end") {
            e.endDate = end.date
        } else if start != nil {
            e.endDate = e.startDate.addingTimeInterval(e.isAllDay ? 0 : length ?? 3600)
        }
        if e.endDate < e.startDate { throw Refusal("end must be after start") }
    }

    /// The event, or for a repeating one, the occurrence, an id names.
    private func event(_ input: Input) throws -> EKEvent {
        let id = try input.text("id")
        let store = Events.shared.store
        if let at = id.lastIndex(of: "@"), let seconds = TimeInterval(id[id.index(after: at)...]) {
            let start = Date(timeIntervalSince1970: seconds)
            let base = String(id[..<at])
            if let e = store.events(matching: store.predicateForEvents(withStart: start, end: start.addingTimeInterval(1), calendars: nil))
                .first(where: { $0.eventIdentifier == base && $0.startDate.timeIntervalSince1970.rounded() == seconds }) {
                return e
            }
        }
        guard let e = store.event(withIdentifier: id) else { throw Refusal("no event with id \(id); calendar_events lists them") }
        return e
    }

    private func json(_ e: EKEvent) -> [String: Any] {
        let repeats = e.hasRecurrenceRules
        let id = e.eventIdentifier ?? ""
        let day = e.isAllDay ? isoDay : iso
        return object([
            "id": repeats ? "\(id)@\(Int(e.startDate.timeIntervalSince1970.rounded()))" : id,
            "title": e.title ?? "", "calendar": e.calendar?.title, "start": day(e.startDate), "end": day(e.endDate),
            "all_day": e.isAllDay ? true : nil, "repeats": repeats ? true : nil,
            "location": e.location?.nonEmpty, "notes": e.notes?.nonEmpty, "url": e.url?.absoluteString,
            "attendees": e.attendees.map { $0.map { $0.name ?? $0.url.absoluteString } },
        ])
    }
}
