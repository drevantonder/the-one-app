import Foundation

struct Brand: Codable, Sendable, Equatable {
    var name: String
    var cli: String
    var accent: String
}

struct NodeState: Codable, Sendable, Equatable {
    struct Device: Codable, Sendable, Equatable {
        var id: String
        var name: String
    }
    var brand: Brand
    var device: Device
    /// The device that answered: this one, or for a phone, the member device acting for it.
    var host: Device?
    /// The one space this app holds, once it creates or joins one.
    var space: Space?
}

struct Space: Codable, Sendable, Equatable, Identifiable {
    var id: String
    var name: String?
    var user: String
    var member: Bool
    var role: String?
    var entries: Int
    var devices: [Member]

    var title: String { name ?? "Joining…" }

    /// This device first, then the online ones, then phones, then by name.
    var sortedDevices: [Member] {
        let rank = { (m: Member) in m.me ? 0 : m.online ? 1 : m.phone ? 2 : 3 }
        return devices.sorted { (rank($0), $0.name.lowercased()) < (rank($1), $1.name.lowercased()) }
    }

    /// The devices that keep a log: phones are never online, so they don't count.
    var nodes: [Member] { devices.filter { !$0.phone } }
}

struct Member: Codable, Sendable, Equatable, Identifiable {
    var device: String
    var name: String
    var user: String
    var role: String
    var me: Bool
    var client: Bool?
    /// For a phone: when it last asked the device that answered, in milliseconds.
    var asked: Double?
    private var isOnline: Bool?

    enum CodingKeys: String, CodingKey {
        case device, name, user, role, me, client, asked
        case isOnline = "online"
    }

    var id: String { device }
    /// A phone keeps no log and holds no sessions, so it's never online: it asks a device that is.
    var phone: Bool { client == true }
    var online: Bool { isOnline == true }

    /// What a shell says of a phone instead of online or offline: when it last asked, if the device that answered knows.
    var phoneStatus: String {
        guard let asked else { return String(localized: "Phone") }
        let date = Date(timeIntervalSince1970: asked / 1000)
        return String(localized: "Phone · asked \(date.formatted(.relative(presentation: .named)))")
    }
}

/// An app as `GET /v1/apps` describes it: only what a shell needs to show it.
struct AppInfo: Decodable, Sendable, Equatable, Identifiable {
    struct Screen: Decodable, Sendable, Equatable {
        var name: String
        var title: String?
    }
    var name: String
    var title: String?
    var icon: String?
    var screens: [Screen]?

    var id: String { name }
    var label: String { title ?? name }
    /// Apps with a `main` screen are the ones a shell lists.
    var hasScreens: Bool { screens?.contains { $0.name == "main" } == true }
}

struct NodeEvent: Codable, Sendable {
    var type: String
    var apps: [String]?
}

struct NodeError: LocalizedError, Sendable {
    var message: String
    var errorDescription: String? { message }

    /// Apps throw JavaScript errors; people only need their first line.
    init(_ raw: String) {
        var line = raw.split(separator: "\n").first.map(String.init) ?? raw
        if line.hasPrefix("Error: ") { line.removeFirst(7) }
        message = line.prefix(1).uppercased() + line.dropFirst()
    }
}

/// How a shell reaches a node: HTTP to this device's own, or, on a phone, the client library to a member device's.
protocol NodeTransport: Sendable {
    /// Sends a request and returns the local API's JSON reply, or throws its error.
    func send(_ method: String, _ path: String, body: Data?, timeout: TimeInterval) async throws -> Data
    /// An app screen's page (SPEC.md "Screens"), and the address it may call back to.
    func page(app: String, screen: String, params: String) async throws -> (html: String, base: URL?)
    /// The node's events, until the connection drops.
    func events() -> AsyncThrowingStream<NodeEvent, Error>
    /// What to say when nothing answers.
    var unreachable: String { get }
}

/// The node's local HTTP API.
struct HTTPNode: NodeTransport {
    var base: URL

    var unreachable: String { "The node isn't answering on \(base.host() ?? ""):\(base.port ?? 0)." }

    func send(_ method: String, _ path: String, body: Data?, timeout: TimeInterval) async throws -> Data {
        var url = base.appending(path: path)
        // A GET's body goes in its query: `{ params }` becomes `?params=`.
        if method == "GET", let body, let fields = try? JSONSerialization.jsonObject(with: body) as? [String: Any] {
            url.append(queryItems: fields.compactMap { k, v in (v as? String).map { URLQueryItem(name: k, value: $0) } })
        }
        var request = URLRequest(url: url, timeoutInterval: timeout)
        request.httpMethod = method
        if let body, method != "GET" {
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.httpBody = body
        }
        let (data, response) = try await URLSession.shared.data(for: request)
        if (response as? HTTPURLResponse)?.statusCode != 200 {
            struct Failure: Decodable { var error: String? }
            let error = (try? JSONDecoder().decode(Failure.self, from: data))?.error
            throw NodeError(error ?? "The node answered \((response as? HTTPURLResponse)?.statusCode ?? 0)")
        }
        return data
    }

    func page(app: String, screen: String, params: String) async throws -> (html: String, base: URL?) {
        let query = try JSONEncoder().encode(["params": params])
        return (String(decoding: try await send("GET", "v1/screens/\(app)/\(screen)", body: query, timeout: 10), as: UTF8.self), base)
    }

    func events() -> AsyncThrowingStream<NodeEvent, Error> {
        let url = base.appending(path: "v1/events")
        return AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    var request = URLRequest(url: url, timeoutInterval: 60)
                    request.setValue("text/event-stream", forHTTPHeaderField: "Accept")
                    let (bytes, _) = try await URLSession.shared.bytes(for: request)
                    for try await line in bytes.lines where line.hasPrefix("data:") {
                        let json = line.dropFirst(5).trimmingCharacters(in: .whitespaces)
                        if let event = try? JSONDecoder().decode(NodeEvent.self, from: Data(json.utf8)) {
                            continuation.yield(event)
                        }
                    }
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: error)
                }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }
}

/// The local API, typed, over whichever transport reaches a node.
struct NodeClient: Sendable {
    var transport: any NodeTransport

    static var local: NodeClient {
        let port = Bundle.main.object(forInfoDictionaryKey: "NodePort") as! String
        return NodeClient(transport: HTTPNode(base: URL(string: "http://127.0.0.1:\(port)")!))
    }

    private struct Empty: Codable {}

    private func send<T: Decodable>(_ method: String, _ path: String, _ body: (any Encodable & Sendable)? = nil, timeout: TimeInterval = 10) async throws -> T {
        let data = try await transport.send(method, path, body: try body.map { try JSONEncoder().encode($0) }, timeout: timeout)
        return try JSONDecoder().decode(T.self, from: data)
    }

    func state() async throws -> NodeState {
        try await send("GET", "v1/state", timeout: 2)
    }

    func apps() async throws -> [AppInfo] {
        try await send("GET", "v1/apps")
    }

    func widgets() async throws -> [WidgetInfo] {
        try await send("GET", "v1/widgets")
    }

    /// Runs an app's operation for an app screen: its input and result are JSON, whatever their shape.
    func call(app: String, op: String, input: Data) async throws -> Data {
        let input = try JSONSerialization.jsonObject(with: input, options: .fragmentsAllowed)
        let body = try JSONSerialization.data(withJSONObject: ["app": app, "op": op, "input": input])
        let data = try await transport.send("POST", "v1/call", body: body, timeout: 15)
        let reply = try JSONSerialization.jsonObject(with: data) as? [String: Any] ?? [:]
        if let error = reply["error"] as? String { throw NodeError(error) }
        return try JSONSerialization.data(withJSONObject: reply["ok"] ?? NSNull(), options: .fragmentsAllowed)
    }

    func page(app: String, screen: String, params: String) async throws -> (html: String, base: URL?) {
        try await transport.page(app: app, screen: screen, params: params)
    }

    func createSpace(_ name: String) async throws {
        struct Body: Encodable, Sendable { var name: String }
        let _: [String: String] = try await send("POST", "v1/spaces", Body(name: name))
    }

    func pair(newUser: Bool) async throws -> String {
        struct Body: Encodable, Sendable { var new_user: Bool }
        let reply: [String: String] = try await send("POST", "v1/pair", Body(new_user: newUser))
        return reply["code"] ?? ""
    }

    func join(_ code: String) async throws {
        struct Body: Encodable, Sendable { var code: String }
        let _: [String: String] = try await send("POST", "v1/join", Body(code: code), timeout: 60)
    }

    func renameDevice(_ name: String) async throws {
        struct Body: Encodable, Sendable { var name: String }
        let _: Empty = try await send("POST", "v1/device", Body(name: name))
    }

    func events() -> AsyncThrowingStream<NodeEvent, Error> {
        transport.events()
    }
}
