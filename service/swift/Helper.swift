import AppKit
import os

/// A Mac service in Swift: a helper app of its own, carried inside the Mac app (Contents/Helpers), which its device's
/// oned starts from a copy outside it (src/oned.rs's `placed`) as a process macOS holds responsible for itself, so
/// macOS asks for, and records, its permission under its own name. oned tells it where to register (`ONE_ADDR`) and where the token it sends is (`ONE_HOME`). It
/// registers only once it has what it needs from the person: until then the Mac app's own copy, if it has one, answers.
public protocol Service: Sendable {
    var app: String { get }
    var description: [String: Any] { get }
    /// What it still needs from the person before it can answer (a permission macOS hasn't given it), in words for
    /// them; nil once it has it. Asked every few seconds until then: asking macOS, which shows its prompt, is done once.
    func needs() async -> String?
    func run(_ function: String, _ input: Input) async throws -> Any
}

/// Runs `service` until oned stops it: an app with no Dock icon or menu, so it can show an alert or a menu bar item.
@MainActor
public func serve(_ service: some Service) -> Never {
    let env = ProcessInfo.processInfo.environment
    guard let at = env["ONE_ADDR"], let home = env["ONE_HOME"] else {
        FileHandle.standardError.write(Data("\(service.app) is a service: its Mac's oned starts it\n".utf8))
        exit(1)
    }
    Node.home = URL(filePath: home)
    let url = URL(string: "http://\(at)/v1/services/\(service.app)")!
    NSApplication.shared.setActivationPolicy(.accessory)
    // Stopped with what started it, even when that was made to stop: two mustn't register as one.
    let parent = getppid()
    Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { _ in if getppid() != parent { exit(0) } }
    Task.detached { await Node.serve(service, at: url) }
    NSApplication.shared.run()
    exit(0)
}

enum Node {
    nonisolated(unsafe) static var home = URL(filePath: "/")

    /// Waits for what it needs, saying it on the service meanwhile (`problem`, which oned shows), then registers and
    /// answers calls, registering again whenever its endpoint forgets it or restarts.
    static func serve(_ service: any Service, at url: URL) async {
        let problem = home.appending(path: "problem")
        while let needs = await service.needs() {
            try? needs.write(to: problem, atomically: true, encoding: .utf8)
            try? await Task.sleep(for: .seconds(3))
        }
        try? FileManager.default.removeItem(at: problem)
        while true {
            do {
                try await post(url, service.description)
                while true {
                    guard let call = try await next(url) else { continue }
                    Task.detached {
                        var reply: [String: Any]
                        do {
                            reply = ["ok": try await service.run(call.function, call.input)]
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

    /// Its own connections: one is held open waiting for calls, and replies mustn't queue behind it.
    private static let session: URLSession = {
        let config = URLSessionConfiguration.ephemeral
        config.httpMaximumConnectionsPerHost = 64
        return URLSession(configuration: config)
    }()

    private struct Call: Sendable {
        var id: UInt64
        var function: String
        var input: Input
    }

    /// Read for each request, so an endpoint that starts after it still lets it in.
    private static func authorize(_ request: inout URLRequest) {
        if let token = try? String(contentsOf: home.appending(path: "local-token"), encoding: .utf8) {
            request.setValue(token.trimmingCharacters(in: .whitespacesAndNewlines), forHTTPHeaderField: "x-one-local-token")
        }
    }

    /// Its endpoint's next call; nil if there was none for 25 seconds.
    private static func next(_ url: URL) async throws -> Call? {
        var request = URLRequest(url: url.appending(path: "calls"), timeoutInterval: 40)
        authorize(&request)
        let (data, response) = try await session.data(for: request)
        let status = (response as? HTTPURLResponse)?.statusCode
        if status == 204 { return nil }
        guard status == 200, let call = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let id = (call["id"] as? NSNumber)?.uint64Value, let function = call["op"] as? String
        else { throw URLError(.badServerResponse) }
        return Call(id: id, function: function, input: Input(call["input"] as? [String: Any] ?? [:], caller: call["caller"] as? [String: Any] ?? [:]))
    }

    private static func post(_ url: URL, _ body: Any) async throws {
        var request = URLRequest(url: url, timeoutInterval: 10)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONSerialization.data(withJSONObject: body)
        authorize(&request)
        let (_, response) = try await session.data(for: request)
        guard (response as? HTTPURLResponse)?.statusCode == 200 else { throw URLError(.badServerResponse) }
    }
}

/// This helper's name, as macOS shows it where it asks and in Privacy & Security ("Contacts — The One App").
public var thisApp: String {
    Bundle.main.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String ?? Bundle.main.object(forInfoDictionaryKey: "CFBundleName") as? String ?? ""
}

/// What a call answers instead of a result, in words for whoever asked.
public struct Refusal: Error, CustomStringConvertible {
    public var description: String
    public init(_ description: String) { self.description = description }
}

/// A call's input, and who made the call (`{ key, user, name, agent }`: the device it came from and the agent that
/// asked through it). Read leniently: the CLI sends a flag that looks like a number as one.
public struct Input: @unchecked Sendable {
    private let raw: [String: Any]
    private let by: [String: Any]
    public init(_ raw: [String: Any], caller: [String: Any] = [:]) { (self.raw, by) = (raw, caller) }

    public func value(_ key: String) -> Any? { raw[key] }

    public func caller(_ key: String) -> String? { (by[key] as? String).flatMap { $0.isEmpty ? nil : $0 } }

    public func has(_ key: String) -> Bool { raw[key] != nil && !(raw[key] is NSNull) }

    public func string(_ key: String) -> String? {
        switch raw[key] {
        case let s as String: s.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? nil : s
        case let n as NSNumber: n.stringValue
        default: nil
        }
    }

    public func text(_ key: String) throws -> String {
        guard let s = string(key) else { throw Refusal("give \(key)") }
        return s
    }

    public func int(_ key: String, or fallback: Int) -> Int {
        (raw[key] as? NSNumber)?.intValue ?? string(key).flatMap { Int($0) } ?? fallback
    }

    public func bool(_ key: String) -> Bool? {
        raw[key] as? Bool ?? string(key).map { $0 == "true" }
    }

    public func date(_ key: String) throws -> When? {
        guard let s = string(key) else { return nil }
        guard let when = When(s) else { throw Refusal("\(key) must be an ISO 8601 date (2026-11-01) or date-time (2026-11-01T09:00 or 2026-11-01T09:00:00+10:00)") }
        return when
    }
}

/// A time a caller gave: a moment, or a whole day.
public struct When {
    public var date: Date
    public var day: Bool

    /// ISO 8601: a date, or a date-time with or without its offset (then it's this Mac's time zone).
    public init?(_ s: String) {
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
public func iso(_ date: Date) -> String {
    let f = ISO8601DateFormatter()
    f.timeZone = .current
    return f.string(from: date)
}

/// A day, as 2026-11-01.
public func isoDay(_ date: Date) -> String {
    let f = ISO8601DateFormatter()
    f.timeZone = .current
    f.formatOptions = [.withFullDate]
    return f.string(from: date)
}

public enum Access { case granted, denied, unasked }

/// Fails a call that needs a permission macOS took back since the helper started, saying how to give it again.
public func permission(_ status: Access, denied: String) throws {
    guard status == .granted else { throw Refusal(denied) }
}

/// What a helper needs while macOS hasn't given it a permission: asks macOS once (its prompt), and says what to do.
public func needs(_ what: String, status: Access, ask: () -> Void, settings: String) -> String? {
    switch status {
    case .granted: return nil
    case .unasked:
        once(what, ask)
        return "\(thisApp) needs \(what). Click Allow where macOS asks on the Mac's screen."
    case .denied:
        return "\(thisApp) needs \(what). Allow it in System Settings → Privacy & Security → \(settings): turn on \(thisApp)."
    }
}

/// Runs `ask` only the first time it's called for `key` while this helper runs.
public func once(_ key: String, _ ask: () -> Void) {
    if asked.withLock({ $0.insert(key).inserted }) { ask() }
}

private let asked = OSAllocatedUnfairLock(initialState: Set<String>())

/// An object JSON leaves out the keys whose values are nil.
public func object(_ fields: [String: Any?]) -> [String: Any] {
    fields.compactMapValues { $0 }
}

/// This helper's build, as the Mac app's build gives it.
public var build: String { Bundle.main.object(forInfoDictionaryKey: "BuildCommit") as? String ?? "" }

/// A service's description: what it is, the Mac app it reaches (`localApp`, a bundle id) and that app's icon as its
/// picture, and the helper's build and how it's installed.
public func describe(title: String, icon: String, localApp: String, summary: String, instructions: String, functions: [[String: Any]]) -> [String: Any] {
    var d: [String: Any] = ["title": title, "icon": icon, "local_app": localApp, "description": summary, "instructions": instructions, "ops": functions, "widgets": [Any](),
                            "version": build, "install": "Comes with the Mac app."]
    if let image = appIcon(localApp) { (d["image"], d["image_kind"]) = (image, "icon") }
    return d
}

/// A Mac app's icon, as a PNG data URL small enough to travel in every Hello: 88 pixels, a 44-point tile at 2x. Only
/// the icon's own shape: macOS draws app icons on a grid that leaves a clear margin (100 of 1024 points on each side),
/// which is cut away, so a shell draws it edge to edge as the Dock does.
public func appIcon(_ bundle: String) -> String? {
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

public func function(_ name: String, _ description: String, _ properties: [String: [String: Any]] = [:], required: [String] = []) -> [String: Any] {
    var input: [String: Any] = ["type": "object", "properties": properties]
    if !required.isEmpty { input["required"] = required }
    return ["name": name, "description": description, "input": input]
}

public func str(_ description: String) -> [String: Any] { ["type": "string", "description": description] }
public func num(_ description: String) -> [String: Any] { ["type": "integer", "description": description] }
public func flag(_ description: String) -> [String: Any] { ["type": "boolean", "description": description] }

extension String {
    public var nonEmpty: String? { isEmpty ? nil : self }
}

/// A framework object that's safe to use from any thread but isn't marked so.
public struct Unshared<T>: @unchecked Sendable {
    public let value: T
    public init(_ value: T) { self.value = value }
}
