import Foundation

struct Config: Codable, Sendable, Equatable {
    var name: String
    var cli: String
    var accent: String
    /// The app the App Store opens: the install's `store` setting.
    var store: String?
    /// The driver phones register their push tokens with: the install's `push` setting, empty when push is off.
    var push: String?
}

/// `GET /v1/state`: this machine, its user and the install.
struct NodeState: Decodable, Sendable, Equatable {
    var config: Config
    /// The machine this shell runs on: this server, or a phone.
    var machine: Machine
    /// The server that answered: this one, or for a phone, the server acting for it.
    var host: Machine?
    /// The user this machine was admitted for, once it is in an install.
    var user: Person?
    var install: Install?
    /// Who opened this user's data, and when.
    var opens: [Open]
    /// How many requests for a Vault item wait for this user; and as nodes before the Vault said it.
    var vaultRequests: Int?
    var secretRequests: Int?
    /// How this user arranged each page, by scope; nil from a server too old to keep arrangements.
    var arrangements: [String: Arrangement]?
    /// What's wrong with backups, for an admin; nil while they work.
    var backups: Backups.Problem?

    var waitingRequests: Int { vaultRequests ?? secretRequests ?? 0 }

    enum CodingKeys: String, CodingKey {
        case config, machine = "me", host = "server", user, install, opens, vaultRequests = "vault_requests", secretRequests = "secret_requests", arrangements, backups
    }
}

/// How one person arranged a page: its apps and the widgets they put there, in order, and the
/// apps they took off it.
struct Arrangement: Codable, Sendable, Equatable {
    struct Item: Codable, Sendable, Equatable, Hashable {
        var app: String
        /// A widget's name, with the id of this copy of it, its size and the options chosen for it; nil for an icon.
        var widget: String?
        var id: String?
        var size: String?
        var options: [String: JSONValue]?
    }
    var items: [Item]
    var hidden: [String]?
}

struct Machine: Codable, Sendable, Equatable {
    var key: String
    var name: String
    /// Where a server serves its skills on the tailnet (its `/llms.txt`), once it does.
    var guide: String?
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
    /// The install's settings, as its admins set them over the white label's defaults.
    var settings: [String: String]?

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
    /// The commit its node was built from.
    var build: String?
    var drivers: [String]?
    /// How loaded it is, as it last said.
    var load: Load?

    var id: String { key }
    var isOnline: Bool { online == true }
    var isPrivileged: Bool { privileged == true }
}

struct Load: Codable, Sendable, Equatable {
    struct Space: Codable, Sendable, Equatable { var used: Double; var total: Double }
    var memory: Space?
    /// Percent busy, oldest first.
    var cpu: [Int]?

    /// "Memory 12.2 of 32 GB · CPU 12%", memory in GB as macOS counts them (2^30 bytes).
    var summary: String? {
        let gb = { (b: Double) in (b / 1_073_741_824).formatted(.number.precision(.fractionLength(1))) }
        var parts: [String] = []
        if let m = memory, m.total > 0 { parts.append(String(localized: "Memory \(gb(m.used)) of \(gb(m.total)) GB")) }
        if let c = cpu?.last { parts.append(String(localized: "CPU \(c)%")) }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }
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
    /// The agent it was opened through, if one did it.
    var agent: String?

    var id: String { "\(by).\(server).\(time)" }
}

/// An app as `GET /v1/apps` describes it: only what a shell needs to show it.
struct AppInfo: Decodable, Sendable, Equatable, Identifiable {
    struct Screen: Decodable, Sendable, Equatable {
        var name: String
        var title: String?
    }
    struct Op: Decodable, Sendable, Equatable {
        var name: String
    }
    struct Widget: Decodable, Sendable, Equatable {
        var name: String
    }
    var name: String
    var title: String?
    /// One line on what it's for.
    var description: String?
    var icon: String?
    /// Its tile's colour, as hex, if it declares one; and its own picture, as a data URL, if it ships one.
    var color: String?
    var image: String?
    /// `icon` when `image` is a finished app icon, drawn edge to edge; else it's a logo, inset on white.
    var imageKind: String?
    var owner: String?
    var instructions: String?
    var screens: [Screen]?
    var ops: [Op]?
    var widgets: [Widget]?
    /// The scopes it is installed in that the asker may use: `user:<id>`, `group:<id>`, `global`.
    var installed: [String]?
    var deploys: Int?
    /// What it asks the Vault for, by name.
    var vault: [String: VaultNeed]?

    enum CodingKeys: String, CodingKey {
        case name, title, description, icon, color, image, imageKind = "image_kind", owner, instructions, screens, ops, widgets, installed, deploys, vault
    }

    var id: String { name }
    var label: String { title ?? name }
    /// Only apps with a `main` screen get an icon; the rest still give widgets and tools.
    var hasScreens: Bool { has(screen: "main") }
    /// A service: nothing to open and no widget, so it goes in a page's Services folder rather than having an icon.
    var isService: Bool { !hasScreens && (widgets ?? []).isEmpty }
    /// Whether it keeps accounts (a connector: `accounts`, `sign_in`, `sign_out`), which its Services folder lists.
    var hasAccounts: Bool { has(op: "accounts") }
    func has(screen: String) -> Bool { screens?.contains { $0.name == screen } == true }
    func has(op: String) -> Bool { ops?.contains { $0.name == op } == true }
    /// Whether its accounts are Vault items its sign-in makes, so Add Account may offer ones that are there.
    var keepsAccounts: Bool { vault?.values.contains { $0.accounts == true } == true }
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
    /// An app screen's page, and the address it may call back to.
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

/// What an app asks the Vault for: a title, the fields it wants, where they're sent, and whether its sign-in makes them.
struct VaultNeed: Decodable, Sendable, Equatable {
    var title: String?
    var accounts: Bool?
    /// The type of the items it's for; the op that makes a new one (given its title); and the ops a person runs on
    /// one (given its id as `item`), by name, with each button's title.
    var type: String?
    var make: String?
    var actions: [String: String]?
}

/// An item of the Vault as `GET /v1/vault` lists it: a secret field never with its value.
struct VaultItem: Decodable, Sendable, Equatable, Identifiable, Hashable {
    struct Field: Decodable, Sendable, Equatable, Hashable, Identifiable {
        var name: String
        var secret: Bool
        var value: String?
        var id: String { name }
    }
    var id: String
    var scope: String
    var scope_name: String?
    var title: String
    var type: String?
    var domain: String?
    var image: String?
    var hosts: [String]
    var fields: [Field]
    /// The apps switched on for it, and those that want it (with the fields they ask for), switched on or not.
    var apps: [String]
    var wanted_by: [String]?
    /// The app whose call made it, and whether it's an account a sign-in made, which signing out deletes.
    var app: String?
    var account: Bool?

    /// Its one short line: what it is, else its first plain field; none when what it is is its title.
    var line: String? {
        if let type, !type.isEmpty { return type == title ? nil : type }
        return shown.first { !$0.secret && !($0.value ?? "").isEmpty }?.value
    }
    /// The fields a person sees: those whose names start with `_` are the app's own, like where it keeps a file.
    var shown: [Field] { fields.filter { !$0.name.hasPrefix("_") } }
    var isAccount: Bool { account == true }

    enum CodingKeys: String, CodingKey { case id, scope, scope_name, title, type, domain, image, hosts, fields, apps, wanted_by, app, account }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        scope = try c.decode(String.self, forKey: .scope)
        title = try c.decode(String.self, forKey: .title)
        hosts = (try? c.decode([String].self, forKey: .hosts)) ?? []
        fields = (try? c.decode([Field].self, forKey: .fields)) ?? []
        apps = (try? c.decode([String].self, forKey: .apps)) ?? []
        scope_name = try? c.decode(String.self, forKey: .scope_name)
        type = try? c.decode(String.self, forKey: .type)
        domain = try? c.decode(String.self, forKey: .domain)
        wanted_by = try? c.decode([String].self, forKey: .wanted_by)
        app = try? c.decode(String.self, forKey: .app)
        account = try? c.decode(Bool.self, forKey: .account)
        // Its icon, as a data URL; a node may give it with its kind, as `[url, kind]`.
        image = (try? c.decode(String.self, forKey: .image)) ?? (try? c.decode([String].self, forKey: .image))?.first
    }
}

/// A request for an item an app may use: the person picks one of theirs or makes one, so the
/// asker never sees its secrets.
struct VaultRequest: Decodable, Sendable, Hashable, Identifiable {
    struct Field: Decodable, Sendable, Equatable, Hashable, Identifiable {
        var name: String
        var secret: Bool
        /// Its value is a file's text (a key's PEM), chosen rather than typed; only a shell's own asks say so.
        var file: Bool?
        var id: String { name }
    }
    struct Match: Decodable, Sendable, Equatable, Hashable, Identifiable {
        var id: String
        var title: String
        var type: String?
        var domain: String?
        var apps: [String]?
    }
    var id: String
    var scope: String
    /// The scope's owner: a user's name, a group's, or the install's.
    var scope_name: String
    var app: String
    var title: String?
    var fields: [Field]?
    /// The one secret it asks for, as nodes before the Vault asked.
    var name: String?
    var hosts: [String]
    var why: String?
    /// The user who asked, and the agent they asked through, if one did.
    var by: String
    var agent: String?
    var time: Double
    /// `waiting`, `set`, `declined` or `expired`.
    var state: String
    var matches: [Match]?

    var waiting: Bool { state == "waiting" }
    var asker: String { agent.flatMap { $0.isEmpty ? nil : $0 } ?? by }
    var label: String { title ?? name ?? "" }
    var wanted: [Field] { fields ?? name.map { [Field(name: $0, secret: true)] } ?? [] }
}

/// Backups, as the node says how they're going.
struct Backups: Decodable, Equatable {
    /// What's wrong with a place, and its fix, in words; since when.
    struct Problem: Decodable, Sendable, Equatable, Hashable { var place: String; var text: String; var fix: String; var since: Double }
    struct Place: Decodable, Equatable, Hashable, Identifiable {
        struct Last: Decodable, Equatable, Hashable { var time: Double; var server: String }
        var id: String
        var title: String
        /// `folder` or `s3`.
        var kind: String
        /// A folder's path, or a bucket.
        var `where`: String
        /// S3's endpoint.
        var endpoint: String?
        var server: String?
        var last: Last?
        var problem: Problem?
    }
    /// A snapshot of the install: whichever servers wrote it, to which places, of whose data.
    struct Snapshot: Decodable, Equatable, Hashable { var time: Double; var servers: [String]; var places: [String]; var scopes: [String] }
    struct Disk: Decodable, Equatable, Identifiable { var name: String; var path: String; var free: Double; var id: String { path } }
    struct Step: Decodable, Equatable, Identifiable { var step: String; var error: String?; var id: String { step } }
    var places: [Place]
    var snapshots: [Snapshot]
    /// The server that answered, whose disks these are.
    var here: String
    var disks: [Disk]
}

/// The local API, typed, over whichever transport reaches a node.
struct NodeClient: Sendable {
    var transport: any NodeTransport
    /// What the node credits this shell's calls to, for apps' insights; a phone's server labels it itself.
    var client: String?

    /// The Mac's own node.
    static var local: NodeClient {
        let port = Bundle.main.object(forInfoDictionaryKey: "NodePort") as! String
        return NodeClient(transport: HTTPNode(base: URL(string: "http://127.0.0.1:\(port)")!), client: "mac")
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

    /// Each app's badge on a page's scope: the count its icon shows. A node too old to know badges has none.
    func badges(scope: String?) async throws -> [String: Int] {
        (try? await get("v1/badges", scope.map { ["scope": $0] } ?? [:])) ?? [:]
    }

    /// Runs an app's operation for an app screen: its input and result are JSON, whatever their shape.
    func call(app: String, op: String, input: Data, timeout: TimeInterval = 15) async throws -> Data {
        let input = try JSONSerialization.jsonObject(with: input, options: .fragmentsAllowed)
        let data = try await transport.send("POST", "v1/call", body: try json(["app": app, "op": op, "input": input, "client": client]), timeout: timeout)
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
    /// Sets one of the install's settings (admins); nil puts back the white label's default.
    func setSetting(_ name: String, _ value: String?) async throws { try await post("v1/settings", ["name": name, "value": value]) }
    func trust(server: String, _ trust: Bool) async throws { try await post("v1/trust", ["server": server, "trust": trust]) }

    // MARK: Groups

    func createGroup(_ name: String) async throws -> String? { try await post("v1/group", ["name": name])["group"] as? String }
    func invite(group: String, user: String, role: String) async throws { try await post("v1/invite", ["group": group, "user": user, "role": role]) }
    func accept(group: String) async throws { try await post("v1/accept", ["group": group]) }
    /// Changes a member's role, or with no role removes them.
    func setMember(group: String, user: String, role: String?) async throws { try await post("v1/member", ["group": group, "user": user, "role": role]) }

    // MARK: Apps, the Vault, backups

    func install(app: String, scope: String, _ installed: Bool) async throws {
        try await post("v1/install", ["app": app, "scope": scope, "installed": installed])
    }

    /// Deletes an app (`deleted`), restores it (`restored`) or erases it (`erased`), for someone acting for its owner.
    func retire(app: String, how: String) async throws {
        try await post("v1/retire", ["app": app, "how": how])
    }

    /// The Vault's items on a page this user manages, or on all of them.
    func vault(scope: String? = nil) async throws -> [VaultItem] { try await get("v1/vault", scope.map { ["scope": $0] } ?? [:]) }

    /// Arranges one of this user's pages, for them alone.
    func arrange(page: String, _ arrangement: Arrangement) async throws {
        let items = try JSONSerialization.jsonObject(with: JSONEncoder().encode(arrangement.items))
        try await post("v1/arrange", ["page": page, "items": items, "hidden": arrangement.hidden ?? []])
    }

    /// Makes an item (no `id`) or changes one: fields as `{ name: { value, secret } }`, `apps`, `hosts`, `deleted`.
    @discardableResult
    func setItem(scope: String, id: String?, _ fields: [String: Any?]) async throws -> String? {
        var body = fields
        body["scope"] = scope
        if let id { body["id"] = id }
        return try await post("v1/vault", body)["id"] as? String
    }

    /// Asks this user for an item an app's need names, for an app screen's `one.vault(need)` → the request's id.
    func requestItem(scope: String, app: String, need: String) async throws -> String? {
        try await post("v1/vault/request", ["scope": scope, "app": app, "need": need])["id"] as? String
    }

    /// Requests for an item this user may answer, newest first: all, or one scope's, or one.
    func vaultRequests(scope: String? = nil, id: String? = nil) async throws -> [VaultRequest] {
        var query: [String: String] = [:]
        query["scope"] = scope
        query["id"] = id
        return try await get("v1/vault/requests", query)
    }

    /// Answers a request: with `item`, switching its app on for that one; else making one of `fields`.
    func answer(request id: String, scope: String, item: String? = nil, title: String? = nil, fields: [String: String]? = nil, declined: Bool = false) async throws {
        var body: [String: Any?] = ["scope": scope, "id": id]
        if declined { body["declined"] = true }
        if let item { body["item"] = item }
        if let title { body["title"] = title }
        if let fields { body["fields"] = fields }
        try await post("v1/vault/answer", body)
    }

    /// Each place backups go to, how its last run went, the recent snapshots, and the disks of the server answering.
    func backups() async throws -> Backups { try await get("v1/backups") }
    /// Backs up now, without waiting for the hour.
    func backUpNow() async throws { try await post("v1/backups/now", [:]) }
    /// Tries a place (writes, reads and deletes a test file) and, if that works, adds it: a folder's `path`, or S3's
    /// `endpoint`, `bucket`, `key_id` and `secret`. → each step tried, and whether it was added.
    func addPlace(_ fields: [String: String]) async throws -> (tried: [Backups.Step], added: Bool) {
        let data = try await transport.send("POST", "v1/backups/place", body: try json(fields), timeout: 60)
        struct Reply: Decodable { var tried: [Backups.Step]; var id: String? }
        let reply = try JSONDecoder().decode(Reply.self, from: data)
        return (reply.tried, reply.id != nil)
    }
    func removePlace(_ id: String) async throws { try await post("v1/backups/place", ["id": id, "deleted": true]) }
}

/// A failed join, said as what went wrong and what to do next, with the node's own words for whoever needs them.
func joinFailure(_ error: Error) -> String {
    let raw = NodeError(error.localizedDescription).message
    let lower = raw.lowercased()
    let next: String
    if lower.contains("already used") || lower.contains("expired") || lower.contains("refused") {
        next = String(localized: "That pairing code has been used or has run out: each works once, for a few minutes. Make a new one on your other device, then try again.")
    } else if lower.contains("already belongs") {
        next = String(localized: "This device is already in an install. Remove it there first, or install the other context’s own app.")
    } else if lower.contains("decode") || lower.contains("invalid") || lower.contains("base") || lower.contains("parse") || lower.contains("bad") {
        next = String(localized: "That isn’t a whole pairing code. Copy it again, all of it, or scan its QR code instead.")
    } else if lower.contains("timed out") || lower.contains("timeout") || lower.contains("connect") || lower.contains("unreachable") || lower.contains("no route") || lower.contains("network") {
        next = String(localized: "Couldn’t reach the device that made the code. Check that it’s on and online (and so is this one), then try again.")
    } else {
        next = String(localized: "Couldn’t join. Make a new pairing code on your other device and try again.")
    }
    return "\(next)\n(\(raw))"
}
