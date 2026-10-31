import Foundation

struct Brand: Codable, Sendable, Equatable {
    var name: String
    var cli: String
    var accent: String
    /// The app the App Store opens, if the brand names one.
    var store: String?
}

/// `GET /v1/state` (SPEC.md "Local API"): this machine, its user and the install.
struct NodeState: Decodable, Sendable, Equatable {
    var brand: Brand
    /// The machine this shell runs on: this server, or a phone.
    var machine: Machine
    /// The server that answered: this one, or for a phone, the server acting for it.
    var host: Machine?
    /// The user this machine was admitted for, once it is in an install.
    var user: Person?
    var install: Install?
    /// Who opened this user's data, and when.
    var opens: [Open]

    enum CodingKeys: String, CodingKey { case brand, machine = "me", host = "server", user, install, opens }
}

struct Machine: Codable, Sendable, Equatable {
    var key: String
    var name: String
}

struct Person: Codable, Sendable, Equatable, Identifiable {
    var id: String
    var name: String
    var role: String

    var admin: Bool { role == "admin" }
}

struct Install: Codable, Sendable, Equatable {
    var name: String
    var users: [Person]
    var groups: [GroupInfo]
    var servers: [Server]
    var clients: [ClientInfo]
    /// Whether the server that answered is in the install yet: a new one isn't until its admit reaches it.
    var member: Bool?

    var joining: Bool { member == false }

    func user(_ id: String) -> Person? { users.first { $0.id == id } }
    func group(_ id: String) -> GroupInfo? { groups.first { $0.id == id } }
}

struct GroupInfo: Codable, Sendable, Equatable, Identifiable {
    struct Member: Codable, Sendable, Equatable, Identifiable {
        var user: String
        var role: String
        var id: String { user }
    }
    var id: String
    var name: String
    var members: [Member]
    var invited: [Member]?

    func role(of user: String) -> String? { members.first { $0.user == user }?.role }
    var admins: [String] { members.filter { $0.role == "admin" }.map(\.user) }
}

struct Server: Codable, Sendable, Equatable, Identifiable {
    var key: String
    var name: String
    /// The user it was admitted for.
    var user: String
    var online: Bool?
    /// `user:<id>`, `group:<id>` or `install`.
    var owners: [String]
    var users: [String]
    var privileged: Bool?
    /// The users who trust it, and so whose data it holds.
    var trusted_by: [String]?
    var target: String?
    var drivers: [String]?

    var id: String { key }
    var isOnline: Bool { online == true }
    var isPrivileged: Bool { privileged == true }
}

struct ClientInfo: Codable, Sendable, Equatable, Identifiable {
    var key: String
    var name: String
    var user: String
    /// When it last asked the server that answered, in milliseconds.
    var asked: Double?

    var id: String { key }
}

struct Open: Codable, Sendable, Equatable, Identifiable {
    /// The user who opened it.
    var by: String
    var server: String
    var time: Double

    var id: String { "\(by).\(server).\(time)" }
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
    var owner: String?
    var instructions: String?
    var screens: [Screen]?
    /// The scopes it is installed in that the asker may use: `user:<id>`, `group:<id>`, `global`.
    var installed: [String]?

    var id: String { name }
    var label: String { title ?? name }
    /// Only apps with a `main` screen get an icon; the rest still give widgets and tools.
    var hasScreens: Bool { screens?.contains { $0.name == "main" } == true }
    /// Its pages: the asker's own scope is the page `user`.
    var scopes: [String] { (installed ?? []).map { $0.hasPrefix("user:") ? "user" : $0 } }
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

/// How a shell reaches a node: HTTP to this machine's own, or, on a phone, the client library to a server.
protocol NodeTransport: Sendable {
    /// Sends a request and returns the local API's JSON reply, or throws its error.
    func send(_ method: String, _ path: String, body: Data?, timeout: TimeInterval) async throws -> Data
    /// An app screen's page (SPEC.md "Screens"), and the address it may call back to.
    func page(app: String, screen: String, params: String, scope: String?) async throws -> (html: String, base: URL?)
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

    func page(app: String, screen: String, params: String, scope: String?) async throws -> (html: String, base: URL?) {
        var query = ["params": params]
        query["scope"] = scope
        let html = try await send("GET", "v1/screens/\(app)/\(screen)", body: try JSONEncoder().encode(query), timeout: 10)
        return (String(decoding: html, as: UTF8.self), base)
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

/// A secret as `GET /v1/secrets` lists it: never its value.
struct SecretInfo: Decodable, Sendable, Equatable, Identifiable {
    var app: String
    var name: String
    var hosts: [String]

    var id: String { "\(app)/\(name)" }
}

/// The local API, typed, over whichever transport reaches a node.
struct NodeClient: Sendable {
    var transport: any NodeTransport

    static var local: NodeClient {
        let port = Bundle.main.object(forInfoDictionaryKey: "NodePort") as! String
        return NodeClient(transport: HTTPNode(base: URL(string: "http://127.0.0.1:\(port)")!))
    }

    private func json(_ fields: [String: Any?]) throws -> Data {
        try JSONSerialization.data(withJSONObject: fields.mapValues { $0 ?? NSNull() })
    }

    private func get<T: Decodable>(_ path: String, _ query: [String: String] = [:], timeout: TimeInterval = 10) async throws -> T {
        let body = query.isEmpty ? nil : try JSONEncoder().encode(query)
        return try JSONDecoder().decode(T.self, from: try await transport.send("GET", path, body: body, timeout: timeout))
    }

    /// A POST whose reply only matters for its fields.
    @discardableResult
    private func post(_ path: String, _ fields: [String: Any?], timeout: TimeInterval = 15) async throws -> [String: Any] {
        let data = try await transport.send("POST", path, body: try json(fields), timeout: timeout)
        return (try? JSONSerialization.jsonObject(with: data) as? [String: Any]) ?? [:]
    }

    func state() async throws -> NodeState { try await get("v1/state", timeout: 2) }
    func apps() async throws -> [AppInfo] { try await get("v1/apps") }

    /// The widgets, rendered for a page's scope.
    func widgets(scope: String?) async throws -> [WidgetInfo] {
        try await get("v1/widgets", scope.map { ["scope": $0] } ?? [:])
    }

    /// Runs an app's operation for an app screen: its input and result are JSON, whatever their shape.
    func call(app: String, op: String, input: Data) async throws -> Data {
        let input = try JSONSerialization.jsonObject(with: input, options: .fragmentsAllowed)
        let data = try await transport.send("POST", "v1/call", body: try json(["app": app, "op": op, "input": input]), timeout: 15)
        let reply = try JSONSerialization.jsonObject(with: data) as? [String: Any] ?? [:]
        if let error = reply["error"] as? String { throw NodeError(error) }
        return try JSONSerialization.data(withJSONObject: reply["ok"] ?? NSNull(), options: .fragmentsAllowed)
    }

    func page(app: String, screen: String, params: String, scope: String?) async throws -> (html: String, base: URL?) {
        try await transport.page(app: app, screen: screen, params: params, scope: scope)
    }

    func events() -> AsyncThrowingStream<NodeEvent, Error> { transport.events() }

    // MARK: The install

    /// Creates an install, with this machine as its first admin's server. The reply's recovery key is shown once.
    func create(name: String, userName: String) async throws -> String? {
        try await post("v1/create", ["name": name, "user_name": userName])["recovery"] as? String
    }

    /// A pairing code: for another of this user's machines, or for someone new (admins only).
    func pair(newUser: (name: String, role: String)? = nil) async throws -> String {
        let fields: [String: Any?] = newUser.map { ["new_user": ["name": $0.name, "role": $0.role]] } ?? [:]
        return try await post("v1/pair", fields)["code"] as? String ?? ""
    }

    func join(_ code: String) async throws { try await post("v1/join", ["code": code], timeout: 60) }

    func rename(_ name: String) async throws { try await post("v1/name", ["name": name]) }

    func remove(machine key: String) async throws { try await post("v1/remove", ["key": key]) }
    func remove(user: String) async throws { try await post("v1/remove", ["user": user]) }
    func setRole(user: String, role: String) async throws { try await post("v1/role", ["user": user, "role": role]) }
    func trust(server: String, _ trust: Bool) async throws { try await post("v1/trust", ["server": server, "trust": trust]) }

    // MARK: Groups

    func createGroup(_ name: String) async throws -> String? { try await post("v1/group", ["name": name])["group"] as? String }
    func invite(group: String, user: String, role: String) async throws { try await post("v1/invite", ["group": group, "user": user, "role": role]) }
    func accept(group: String) async throws { try await post("v1/accept", ["group": group]) }
    /// Changes a member's role, or with no role removes them.
    func setMember(group: String, user: String, role: String?) async throws { try await post("v1/member", ["group": group, "user": user, "role": role]) }

    // MARK: Apps, secrets, backups

    func install(app: String, scope: String, _ installed: Bool) async throws {
        try await post("v1/install", ["app": app, "scope": scope, "installed": installed])
    }

    func secrets(scope: String) async throws -> [SecretInfo] { try await get("v1/secrets", ["scope": scope]) }

    /// Sets a secret, or with no value deletes it.
    func setSecret(scope: String, app: String, name: String, hosts: [String], value: String?) async throws {
        try await post("v1/secrets", ["scope": scope, "app": app, "name": name, "hosts": hosts, "value": value])
    }

    func backup(url: String?, accessKey: String = "", secretKey: String = "", key: String = "") async throws {
        if let url {
            try await post("v1/backup", ["url": url, "access_key": accessKey, "secret_key": secretKey, "key": key])
        } else {
            try await post("v1/backup", ["url": nil])
        }
    }
}
