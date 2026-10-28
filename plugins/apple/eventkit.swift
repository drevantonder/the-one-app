// one-eventkit: the Mac side of the reminders and calendar plugins.
// Usage: one-eventkit <command> < input.json   →   {"result": …} or {"error": "…"}
import EventKit
import Foundation

let store = EKEventStore()
let iso = ISO8601DateFormatter()
iso.formatOptions = [.withInternetDateTime]
iso.timeZone = .current
let dayOnly = ISO8601DateFormatter()
dayOnly.formatOptions = [.withFullDate]
dayOnly.timeZone = .current

func fail(_ msg: String) -> Never {
    let out = try! JSONSerialization.data(withJSONObject: ["error": msg])
    FileHandle.standardOutput.write(out)
    exit(1)
}

func done(_ result: Any) -> Never {
    let out = try! JSONSerialization.data(withJSONObject: ["result": result], options: [.sortedKeys])
    FileHandle.standardOutput.write(out)
    exit(0)
}

func date(_ v: Any?) -> Date? {
    guard let s = v as? String, !s.isEmpty else { return nil }
    return iso.date(from: s) ?? dayOnly.date(from: s)
}

func access(_ type: EKEntityType) {
    let sem = DispatchSemaphore(value: 0)
    var ok = false
    let handler: (Bool, Error?) -> Void = { granted, _ in ok = granted; sem.signal() }
    if type == .reminder { store.requestFullAccessToReminders(completion: handler) } else { store.requestFullAccessToEvents(completion: handler) }
    if sem.wait(timeout: .now() + 60) == .timedOut {
        fail("waiting for macOS permission: allow access in the prompt on the Mac's screen, or in System Settings → Privacy & Security")
    }
    if !ok { fail("macOS denied access to \(type == .reminder ? "Reminders" : "Calendar"). Allow it in System Settings → Privacy & Security.") }
}

func calendar(_ type: EKEntityType, _ name: Any?) -> EKCalendar? {
    guard let name = name as? String, !name.isEmpty else { return nil }
    if let c = store.calendars(for: type).first(where: { $0.title == name || $0.calendarIdentifier == name }) { return c }
    fail("no \(type == .reminder ? "list" : "calendar") called \(name)")
}

func reminderJSON(_ r: EKReminder) -> [String: Any] {
    var o: [String: Any] = ["id": r.calendarItemIdentifier, "title": r.title ?? "", "list": r.calendar.title, "completed": r.isCompleted, "priority": r.priority]
    if let c = r.dueDateComponents, let d = Calendar.current.date(from: c) {
        // A reminder due on a day, with no time, reads as that day.
        o["due"] = c.hour == nil ? dayOnly.string(from: d) : iso.string(from: d)
    }
    if let n = r.notes, !n.isEmpty { o["notes"] = n }
    if let c = r.completionDate { o["completedAt"] = iso.string(from: c) }
    return o
}

func eventJSON(_ e: EKEvent) -> [String: Any] {
    var o: [String: Any] = ["id": e.eventIdentifier ?? "", "title": e.title ?? "", "calendar": e.calendar.title,
                            "start": iso.string(from: e.startDate), "end": iso.string(from: e.endDate), "allDay": e.isAllDay]
    if let l = e.location, !l.isEmpty { o["location"] = l }
    if let n = e.notes, !n.isEmpty { o["notes"] = n }
    return o
}

func fetchReminders(_ predicate: NSPredicate) -> [EKReminder] {
    let sem = DispatchSemaphore(value: 0)
    var out: [EKReminder] = []
    store.fetchReminders(matching: predicate) { out = $0 ?? []; sem.signal() }
    sem.wait()
    return out
}

func reminder(_ id: Any?) -> EKReminder {
    guard let id = id as? String, let r = store.calendarItem(withIdentifier: id) as? EKReminder else { fail("no reminder with that id") }
    return r
}

func applyDue(_ r: EKReminder, _ v: Any?) {
    guard let v = v as? String else { return }
    if v.isEmpty { r.dueDateComponents = nil; return }
    guard let d = date(v) else { fail("due must be an ISO 8601 date or date-time") }
    let parts: Set<Calendar.Component> = v.count <= 10 ? [.year, .month, .day] : [.year, .month, .day, .hour, .minute]
    r.dueDateComponents = Calendar.current.dateComponents(parts, from: d)
}

let args = CommandLine.arguments
guard args.count == 2 else { fail("usage: one-eventkit <command> < input.json") }
let raw = FileHandle.standardInput.readDataToEndOfFile()
let input = (raw.isEmpty ? [:] : (try? JSONSerialization.jsonObject(with: raw)) as? [String: Any]) ?? [:]

do {
    switch args[1] {
    case "reminders.lists":
        access(.reminder)
        done(store.calendars(for: .reminder).map { ["name": $0.title, "id": $0.calendarIdentifier] })
    case "reminders.list":
        access(.reminder)
        let lists = calendar(.reminder, input["list"]).map { [$0] }
        let showDone = input["completed"] as? Bool ?? false
        let predicate = showDone ? store.predicateForReminders(in: lists) : store.predicateForIncompleteReminders(withDueDateStarting: nil, ending: nil, calendars: lists)
        let limit = input["limit"] as? Int ?? 200
        done(Array(fetchReminders(predicate).map(reminderJSON).prefix(limit)))
    case "reminders.create":
        access(.reminder)
        guard let title = input["title"] as? String, !title.isEmpty else { fail("title is required") }
        let r = EKReminder(eventStore: store)
        r.title = title
        r.calendar = calendar(.reminder, input["list"]) ?? store.defaultCalendarForNewReminders()
        r.notes = input["notes"] as? String
        if let p = input["priority"] as? Int { r.priority = p }
        applyDue(r, input["due"])
        try store.save(r, commit: true)
        done(reminderJSON(r))
    case "reminders.update":
        access(.reminder)
        let r = reminder(input["id"])
        if let t = input["title"] as? String { r.title = t }
        if let n = input["notes"] as? String { r.notes = n }
        if let p = input["priority"] as? Int { r.priority = p }
        if let c = input["completed"] as? Bool { r.isCompleted = c }
        if input["list"] != nil, let l = calendar(.reminder, input["list"]) { r.calendar = l }
        applyDue(r, input["due"])
        try store.save(r, commit: true)
        done(reminderJSON(r))
    case "reminders.delete":
        access(.reminder)
        try store.remove(reminder(input["id"]), commit: true)
        done("Deleted.")
    case "calendar.calendars":
        access(.event)
        done(store.calendars(for: .event).map { ["name": $0.title, "id": $0.calendarIdentifier, "writable": $0.allowsContentModifications] })
    case "calendar.events":
        access(.event)
        let from = date(input["from"]) ?? Calendar.current.startOfDay(for: Date())
        let to = date(input["to"]) ?? from.addingTimeInterval(7 * 86400)
        let cals = calendar(.event, input["calendar"]).map { [$0] }
        var events = store.events(matching: store.predicateForEvents(withStart: from, end: to, calendars: cals))
        if let q = (input["query"] as? String)?.lowercased(), !q.isEmpty {
            events = events.filter { ($0.title ?? "").lowercased().contains(q) || ($0.location ?? "").lowercased().contains(q) || ($0.notes ?? "").lowercased().contains(q) }
        }
        done(events.sorted { $0.startDate < $1.startDate }.map(eventJSON))
    case "calendar.create":
        access(.event)
        guard let title = input["title"] as? String, !title.isEmpty else { fail("title is required") }
        guard let start = date(input["start"]) else { fail("start is required (ISO 8601)") }
        let e = EKEvent(eventStore: store)
        e.title = title
        e.startDate = start
        e.isAllDay = input["allDay"] as? Bool ?? false
        e.endDate = date(input["end"]) ?? start.addingTimeInterval(e.isAllDay ? 86400 : 3600)
        e.calendar = calendar(.event, input["calendar"]) ?? store.defaultCalendarForNewEvents
        e.location = input["location"] as? String
        e.notes = input["notes"] as? String
        try store.save(e, span: .thisEvent, commit: true)
        done(eventJSON(e))
    case "calendar.delete":
        access(.event)
        guard let id = input["id"] as? String, let e = store.event(withIdentifier: id) else { fail("no event with that id") }
        try store.remove(e, span: .thisEvent, commit: true)
        done("Deleted.")
    default:
        fail("unknown command \(args[1])")
    }
} catch {
    fail(error.localizedDescription)
}
