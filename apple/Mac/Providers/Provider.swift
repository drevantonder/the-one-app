import Foundation

/// A device feature this app offers the person's devices through this device's node (SPEC.md, "Local API").
/// Apple's apps are provided from here, not the node, because this app is what macOS grants privacy permissions to.
protocol Provider: Sendable {
    var app: String { get }
    var description: [String: Any] { get }
    func run(_ op: String, _ input: Input) async throws -> Any
}

enum Providers {
    static let all: [any Provider] = [RemindersApp(), CalendarApp(), ContactsApp(), MessagesApp()]

    static func start() {
        let port = Bundle.main.object(forInfoDictionaryKey: "NodePort") as! String
        let base = URL(string: "http://127.0.0.1:\(port)/v1/providers")!
        for provider in all {
            Task.detached { await serve(provider, at: base.appending(path: provider.app)) }
        }
    }

    /// Registers with the node and answers its calls, registering again whenever the node forgets (it keeps
    /// providers in memory) or restarts.
    private static func serve(_ provider: any Provider, at url: URL) async {
        while true {
            do {
                try await post(url, provider.description)
                while true {
                    guard let call = try await next(url) else { continue }
                    Task.detached {
                        var reply: [String: Any]
                        do {
                            reply = ["ok": try await provider.run(call.op, call.input)]
                        } catch {
                            reply = ["error": (error as? Refusal)?.description ?? error.localizedDescription]
                        }
                        try? await post(url.appending(path: "calls/\(call.id)"), reply)
                    }
                }
            } catch {
                try? await Task.sleep(for: .seconds(3))
            }
        }
    }

    private struct Call: Sendable {
        var id: UInt64
        var op: String
        var input: Input
    }

    /// The node's next call; nil if there was none for 25 seconds.
    private static func next(_ url: URL) async throws -> Call? {
        let (data, response) = try await URLSession.shared.data(for: URLRequest(url: url.appending(path: "calls"), timeoutInterval: 40))
        let status = (response as? HTTPURLResponse)?.statusCode
        if status == 204 { return nil }
        guard status == 200, let call = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let id = (call["id"] as? NSNumber)?.uint64Value, let op = call["op"] as? String
        else { throw URLError(.badServerResponse) }
        return Call(id: id, op: op, input: Input(call["input"] as? [String: Any] ?? [:]))
    }

    private static func post(_ url: URL, _ body: Any) async throws {
        var request = URLRequest(url: url, timeoutInterval: 10)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONSerialization.data(withJSONObject: body)
        let (_, response) = try await URLSession.shared.data(for: request)
        guard (response as? HTTPURLResponse)?.statusCode == 200 else { throw URLError(.badServerResponse) }
    }
}

/// What a call answers instead of a result, in words for whoever asked.
struct Refusal: Error, CustomStringConvertible {
    var description: String
    init(_ description: String) { self.description = description }
}

/// A call's input. Read leniently: the CLI sends a flag that looks like a number as one.
struct Input: @unchecked Sendable {
    private let raw: [String: Any]
    init(_ raw: [String: Any]) { self.raw = raw }

    func has(_ key: String) -> Bool { raw[key] != nil && !(raw[key] is NSNull) }

    func string(_ key: String) -> String? {
        switch raw[key] {
        case let s as String: s.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? nil : s
        case let n as NSNumber: n.stringValue
        default: nil
        }
    }

    func text(_ key: String) throws -> String {
        guard let s = string(key) else { throw Refusal("give \(key)") }
        return s
    }

    func int(_ key: String, or fallback: Int) -> Int {
        (raw[key] as? NSNumber)?.intValue ?? string(key).flatMap { Int($0) } ?? fallback
    }

    func bool(_ key: String) -> Bool? {
        raw[key] as? Bool ?? string(key).map { $0 == "true" }
    }

    func date(_ key: String) throws -> When? {
        guard let s = string(key) else { return nil }
        guard let when = When(s) else { throw Refusal("\(key) must be an ISO 8601 date (2026-11-01) or date-time (2026-11-01T09:00 or 2026-11-01T09:00:00+10:00)") }
        return when
    }
}

/// A time a caller gave: a moment, or a whole day.
struct When {
    var date: Date
    var day: Bool

    /// ISO 8601: a date, or a date-time with or without its offset (then it's this Mac's time zone).
    init?(_ s: String) {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.timeZone = .current
        let s = s.replacingOccurrences(of: " ", with: "T")
        for (format, day) in [("yyyy-MM-dd'T'HH:mm:ss.SSSXXXXX", false), ("yyyy-MM-dd'T'HH:mm:ssXXXXX", false), ("yyyy-MM-dd'T'HH:mmXXXXX", false),
                              ("yyyy-MM-dd'T'HH:mm:ss", false), ("yyyy-MM-dd'T'HH:mm", false), ("yyyy-MM-dd", true)] {
            f.dateFormat = format
            if let date = f.date(from: s) {
                self.date = date
                self.day = day
                return
            }
        }
        return nil
    }
}

/// A moment in this Mac's time zone, as 2026-11-01T09:00:00+10:00.
func iso(_ date: Date) -> String {
    let f = ISO8601DateFormatter()
    f.timeZone = .current
    return f.string(from: date)
}

/// A day, as 2026-11-01.
func isoDay(_ date: Date) -> String {
    let f = ISO8601DateFormatter()
    f.timeZone = .current
    f.formatOptions = [.withFullDate]
    return f.string(from: date)
}

enum Access { case granted, denied, unasked }

/// Asks macOS for a permission on the first call that needs it. Nobody may be at the Mac to answer the
/// prompt, so the call waits briefly, then fails saying what to allow, rather than holding on.
func permission(_ what: String, status: () -> Access, ask: () -> Void, denied: String) async throws {
    if status() == .unasked {
        ask()
        let deadline = Date.now.addingTimeInterval(15)
        while status() == .unasked, Date.now < deadline {
            try await Task.sleep(for: .milliseconds(250))
        }
    }
    switch status() {
    case .granted: return
    case .denied: throw Refusal(denied)
    case .unasked: throw Refusal("macOS is asking on the Mac's screen whether \(appName) may use \(what). Click Allow there, then try again.")
    }
}

/// An object JSON leaves out the keys whose values are nil.
func object(_ fields: [String: Any?]) -> [String: Any] {
    fields.compactMapValues { $0 }
}

func describe(title: String, icon: String, instructions: String, ops: [[String: Any]]) -> [String: Any] {
    ["title": title, "icon": icon, "instructions": instructions, "ops": ops, "widgets": [Any]()]
}

func op(_ name: String, _ description: String, _ properties: [String: [String: Any]] = [:], required: [String] = []) -> [String: Any] {
    var input: [String: Any] = ["type": "object", "properties": properties]
    if !required.isEmpty { input["required"] = required }
    return ["name": name, "description": description, "input": input]
}

func str(_ description: String) -> [String: Any] { ["type": "string", "description": description] }
func num(_ description: String) -> [String: Any] { ["type": "integer", "description": description] }
func flag(_ description: String) -> [String: Any] { ["type": "boolean", "description": description] }
