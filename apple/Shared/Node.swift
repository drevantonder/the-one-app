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
    var spaces: [Space]
}

struct Space: Codable, Sendable, Equatable, Identifiable {
    var id: String
    var name: String?
    var person: String
    var member: Bool
    var role: String?
    var entries: Int
    var devices: [Member]

    var title: String { name ?? "Joining…" }

    /// This device first, then the online ones, then by name.
    var sortedDevices: [Member] {
        devices.sorted { ($0.me ? 0 : $0.online ? 1 : 2, $0.name.lowercased()) < ($1.me ? 0 : $1.online ? 1 : 2, $1.name.lowercased()) }
    }
}

struct Member: Codable, Sendable, Equatable, Identifiable {
    var device: String
    var name: String
    var person: String
    var role: String
    var me: Bool
    var online: Bool

    var id: String { device }
}

struct Post: Codable, Sendable, Equatable, Identifiable {
    var id: String
    var topic: String
    var text: String
    var name: String
    var person: String
    var time: Int64

    var date: Date { Date(timeIntervalSince1970: TimeInterval(time) / 1000) }
}

struct Topic: Codable, Sendable, Equatable, Identifiable {
    var path: String
    var description: String
    var posts: Int
    var last: Int64?

    var id: String { path }
    var name: String { path.split(separator: "/").last.map(String.init) ?? path }
    var parent: String { path.contains("/") ? String(path[..<path.lastIndex(of: "/")!]) : "" }
}

struct BoardPage: Codable, Sendable, Equatable {
    var topic: String
    var description: String
    var posts: [Post]
    var subtopics: [Topic]
}

struct NodeEvent: Decodable, Sendable {
    var type: String
    var space: String?
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

/// The node's local HTTP API.
struct NodeClient: Sendable {
    var base: URL

    static let local: NodeClient = {
        let port = Bundle.main.object(forInfoDictionaryKey: "NodePort") as! String
        return NodeClient(base: URL(string: "http://127.0.0.1:\(port)")!)
    }()

    private struct Reply<T: Decodable>: Decodable {
        var ok: T?
        var error: String?
    }

    private struct Empty: Codable {}

    private struct Call: Encodable, Sendable {
        var space: String?
        var app: String
        var op: String
        var input: [String: JSONValue]
    }

    private func send<T: Decodable>(_ method: String, _ path: String, _ body: (any Encodable & Sendable)? = nil, timeout: TimeInterval = 10) async throws -> T {
        var request = URLRequest(url: base.appending(path: path), timeoutInterval: timeout)
        request.httpMethod = method
        if let body {
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.httpBody = try JSONEncoder().encode(body)
        }
        let (data, response) = try await URLSession.shared.data(for: request)
        if (response as? HTTPURLResponse)?.statusCode != 200 {
            let error = (try? JSONDecoder().decode(Reply<Empty>.self, from: data))?.error
            throw NodeError(error ?? "The node answered \((response as? HTTPURLResponse)?.statusCode ?? 0)")
        }
        return try JSONDecoder().decode(T.self, from: data)
    }

    func state() async throws -> NodeState {
        try await send("GET", "v1/state", timeout: 2)
    }

    func call<T: Decodable>(_ app: String, _ op: String, space: String?, input: [String: JSONValue] = [:]) async throws -> T {
        let reply: Reply<T> = try await send("POST", "v1/call", Call(space: space, app: app, op: op, input: input))
        guard let ok = reply.ok else { throw NodeError(reply.error ?? "No answer") }
        return ok
    }

    func createSpace(_ name: String) async throws {
        struct Body: Encodable, Sendable { var name: String }
        let _: [String: String] = try await send("POST", "v1/spaces", Body(name: name))
    }

    func invite(space: String, mine: Bool) async throws -> String {
        struct Body: Encodable, Sendable { var space: String; var mine: Bool }
        let reply: [String: String] = try await send("POST", "v1/invite", Body(space: space, mine: mine))
        return reply["ticket"] ?? ""
    }

    func join(_ ticket: String) async throws {
        struct Body: Encodable, Sendable { var ticket: String }
        let _: [String: String] = try await send("POST", "v1/join", Body(ticket: ticket), timeout: 60)
    }

    func renameDevice(_ name: String) async throws {
        struct Body: Encodable, Sendable { var name: String }
        let _: Empty = try await send("POST", "v1/device", Body(name: name))
    }

    /// The node's server-sent events, until the connection drops.
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

enum JSONValue: Codable, Sendable, Equatable, ExpressibleByStringLiteral, ExpressibleByIntegerLiteral {
    case string(String)
    case number(Double)

    init(stringLiteral value: String) { self = .string(value) }
    init(integerLiteral value: Int) { self = .number(Double(value)) }

    init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if let s = try? c.decode(String.self) { self = .string(s) } else { self = .number(try c.decode(Double.self)) }
    }

    func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .string(let s): try c.encode(s)
        case .number(let n): try c.encode(n)
        }
    }
}
