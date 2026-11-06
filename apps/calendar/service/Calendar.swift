@preconcurrency import EventKit
import Foundation
import Helper

struct CalendarApp: Service {
    let app = "calendar"
    let localApp: String? = "com.apple.iCal"

    func needs() async -> String? { Events.shared.needs(.event) }

    func run(_ function: String, _ input: Input) async throws -> Any {
        let ek = Events.shared, store = ek.store
        try ek.access(.event)
        switch function {
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
            throw Refusal("calendar has no function \(function)")
        }
    }

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
