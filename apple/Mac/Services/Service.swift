import AppKit

/// A device service the Mac app hosts: a device feature it offers the person's devices through this Mac's node.
/// Apple's apps are reached from here, not the node, because this app is what macOS grants
/// privacy permissions to.
protocol Service: Sendable {
    var app: String { get }
    var description: [String: Any] { get }
    func run(_ op: String, _ input: Input) async throws -> Any
}

enum Services {
    static let all: [any Service] = [RemindersApp(), CalendarApp(), ContactsApp(), MessagesApp(), ComputerApp()]

    static func start() {
        let port = Bundle.main.object(forInfoDictionaryKey: "NodePort") as! String
        let base = URL(string: "http://127.0.0.1:\(port)/v1/services")!
        for service in all {
            Task.detached { await serve(service, at: base.appending(path: service.app)) }
        }
    }

    /// Registers with the node and answers its calls, registering again whenever the node forgets (it keeps
    /// services in memory) or restarts.
    private static func serve(_ service: any Service, at url: URL) async {
        while true {
            do {
                try await post(url, service.description)
                while true {
                    guard let call = try await next(url) else { continue }
                    Task.detached {
                        var reply: [String: Any]
                        do {
                            reply = ["ok": try await service.run(call.op, call.input)]
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

    /// The services' own connections to the node: each holds one open waiting for calls, and URLSession.shared allows
    /// six to a host, so sharing it would leave the window's requests queued behind them until they time out.
    private static let session: URLSession = {
        let config = URLSessionConfiguration.ephemeral
        config.httpMaximumConnectionsPerHost = 64
        return URLSession(configuration: config)
    }()

    private struct Call: Sendable {
        var id: UInt64
        var op: String
        var input: Input
    }

    /// The node's next call; nil if there was none for 25 seconds.
    private static func next(_ url: URL) async throws -> Call? {
        var request = URLRequest(url: url.appending(path: "calls"), timeoutInterval: 40)
        LocalToken.add(to: &request)
        let (data, response) = try await session.data(for: request)
        let status = (response as? HTTPURLResponse)?.statusCode
        if status == 204 { return nil }
        guard status == 200, let call = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let id = (call["id"] as? NSNumber)?.uint64Value, let op = call["op"] as? String
        else { throw URLError(.badServerResponse) }
        return Call(id: id, op: op, input: Input(call["input"] as? [String: Any] ?? [:], caller: call["caller"] as? [String: Any] ?? [:]))
    }

    private static func post(_ url: URL, _ body: Any) async throws {
        var request = URLRequest(url: url, timeoutInterval: 10)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONSerialization.data(withJSONObject: body)
        LocalToken.add(to: &request)
        let (_, response) = try await session.data(for: request)
        guard (response as? HTTPURLResponse)?.statusCode == 200 else { throw URLError(.badServerResponse) }
    }
}

/// What a call answers instead of a result, in words for whoever asked.
struct Refusal: Error, CustomStringConvertible {
    var description: String
    init(_ description: String) { self.description = description }
}

/// A call's input, and who made the call (`{ key, user, name, agent }`: the device it came from and the agent that
/// asked through it). Read leniently: the CLI sends a flag that looks like a number as one.
struct Input: @unchecked Sendable {
    private let raw: [String: Any]
    private let by: [String: Any]
    init(_ raw: [String: Any], caller: [String: Any] = [:]) { (self.raw, by) = (raw, caller) }

    func value(_ key: String) -> Any? { raw[key] }

    func caller(_ key: String) -> String? { (by[key] as? String).flatMap { $0.isEmpty ? nil : $0 } }

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

/// A service's description: what it is, the Mac app it reaches (`localApp`, a bundle id) and that app's icon as its
/// picture, and the Mac app's build and how it's installed.
func describe(title: String, icon: String, localApp: String, summary: String, instructions: String, ops: [[String: Any]]) -> [String: Any] {
    var d: [String: Any] = ["title": title, "icon": icon, "local_app": localApp, "description": summary, "instructions": instructions, "ops": ops, "widgets": [Any](),
                            "version": Bundle.main.object(forInfoDictionaryKey: "BuildCommit") as? String ?? "", "install": "Comes with the Mac app, on each Mac it's open on."]
    if let image = appIcon(localApp) { (d["image"], d["image_kind"]) = (image, "icon") }
    return d
}

/// A Mac app's icon, as a PNG data URL small enough to travel in every Hello: 88 pixels, a 44-point tile at 2x. Only
/// the icon's own shape: macOS draws app icons on a grid that leaves a clear margin (100 of 1024 points on each side),
/// which is cut away, so a shell draws it edge to edge as the Dock does.
func appIcon(_ bundle: String) -> String? {
    guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundle) else { return nil }
    let icon = NSWorkspace.shared.icon(forFile: url.path)
    let size = 88
    let full = CGFloat(size) * 1024 / 824, margin = (full - CGFloat(size)) / 2
    guard let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true,
                                     isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0),
          let context = NSGraphicsContext(bitmapImageRep: rep) else { return nil }
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = context
    icon.draw(in: NSRect(x: -margin, y: -margin, width: full, height: full))
    NSGraphicsContext.restoreGraphicsState()
    guard let png = rep.representation(using: .png, properties: [:]) else { return nil }
    return "data:image/png;base64," + png.base64EncodedString()
}

func op(_ name: String, _ description: String, _ properties: [String: [String: Any]] = [:], required: [String] = []) -> [String: Any] {
    var input: [String: Any] = ["type": "object", "properties": properties]
    if !required.isEmpty { input["required"] = required }
    return ["name": name, "description": description, "input": input]
}

func str(_ description: String) -> [String: Any] { ["type": "string", "description": description] }
func num(_ description: String) -> [String: Any] { ["type": "integer", "description": description] }
func flag(_ description: String) -> [String: Any] { ["type": "boolean", "description": description] }
