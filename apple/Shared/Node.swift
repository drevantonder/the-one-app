import Foundation

struct Config: Codable, Sendable, Equatable {
    var name: String
    var cli: String
    /// The app the App Store opens: the network's `store` setting.
    var store: String?
}

/// `GET /v1/state`: this machine, its user and the network.
struct NodeState: Decodable, Sendable, Equatable {
    var config: Config
    /// The machine this shell runs on: this device, or a phone.
    var machine: Machine
    /// The device that answered: this one, or for a phone, the device acting for it.
    var host: Machine?
    /// The user this machine was admitted for, once it is in a network.
    var user: Person?
    var install: Install?
    /// How many requests for a Vault item wait for this user.
    var vaultRequests: Int?
    /// How this user arranged each page, by scope; nil from a device too old to keep arrangements.
    var arrangements: [String: Arrangement]?
    /// What's wrong with backups, for an admin; nil while they work.
    var backups: Backups.Problem?
    /// What Settings shows this user, in order; nil from a device too old to say.
    var settingsItems: [SettingsItem]?
    /// What to paste into an agent so it connects itself; nil from a device too old to say.
    var agentPrompt: String?

    var waitingRequests: Int { vaultRequests ?? 0 }

    enum CodingKeys: String, CodingKey {
        case config, machine = "me", host = "device", user, install, vaultRequests = "vault_requests", arrangements, backups
        case settingsItems = "settings_items", agentPrompt = "agent_prompt"
    }
}

/// One of Settings' items: it opens the Settings screen it names, or with `app`, that app's main screen.
struct SettingsItem: Decodable, Sendable, Equatable, Identifiable {
    var name: String
    var title: String
    /// A Phosphor icon's name, on `color`.
    var icon: String
    var color: String
    var badge: Int?
    var app: String?
    /// Where it's opened in a browser, in place of a screen (Passkeys, at the network's address).
    var url: URL?

    var id: String { name }
}

/// How one person arranged a page, which the shared page keeps: its apps and the widgets they put there, in order;
/// and on their own page, how they ordered their groups' pages and which they hid.
struct Arrangement: Codable, Sendable, Equatable {
    struct Item: Codable, Sendable, Equatable {
        var app: String
        /// A widget's name, with the id of this copy of it, its size and the options chosen for it; nil for an icon.
        var widget: String?
        var id: String?
        var size: String?
        var options: [String: JSONValue]?
    }
    struct Pages: Codable, Sendable, Equatable {
        var order: [String]?
        var hidden: [String]?
    }
    var items: [Item]
    var pages: Pages?
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
    var devices: [Device]
    var clients: [ClientInfo]
    /// Whether the device that answered is in the network yet: a new one isn't until its admit reaches it.
    var member: Bool?
    /// The network's settings, as its admins set them over the custom build's defaults.
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

struct Device: Codable, Sendable, Equatable, Identifiable {
    var key: String
    var name: String
    /// The person it's listed as a device of.
    var user: String
    var online: Bool?
    /// The scopes whose calls it runs (`user:<id>`, `group:<id>`, `global`).
    var runs_for: [String]?
    /// For a service endpoint, the service it offers (`whatsapp`); its `name` is its machine's.
    var service: String?
    var target: String?

    var id: String { key }
    var isOnline: Bool { online == true }
}

struct ClientInfo: Codable, Sendable, Equatable, Identifiable {
    var key: String
    var name: String
    var user: String

    var id: String { key }
}

/// An app as `GET /v1/apps` describes it: only what a shell needs to show it.
struct AppInfo: Decodable, Sendable, Equatable, Identifiable {
    struct Screen: Decodable, Sendable, Equatable {
        var name: String
        var title: String?
    }
    struct Widget: Decodable, Sendable, Equatable {
        var name: String
    }
    struct Tool: Decodable, Sendable, Equatable {
        var name: String
    }
    var name: String
    var title: String?
    var icon: String?
    /// Its tile's colour, as hex, if it declares one; and its own picture, as a data URL, if it ships one.
    var color: String?
    var image: String?
    /// `icon` when `image` is a finished app icon, drawn edge to edge; else it's a logo, inset on white.
    var imageKind: String?
    var screens: [Screen]?
    var widgets: [Widget]?
    var ops: [Tool]?
    /// The scopes it is installed in that the asker may use: `user:<id>`, `group:<id>`, `global`.
    var installed: [String]?
    var deploys: Int?

    enum CodingKeys: String, CodingKey {
        case name, title, icon, color, image, imageKind = "image_kind", screens, widgets, ops, installed, deploys
    }

    var id: String { name }
    var label: String { title ?? name }
    /// Only apps with a `main` screen get an icon; the rest still give widgets and tools.
    var hasScreens: Bool { has(screen: "main") }
    /// It signs in to accounts (it has an `accounts` tool), so it's in its page's Accounts.
    var keepsAccounts: Bool { ops?.contains { $0.name == "accounts" } == true }
    func has(screen: String) -> Bool { screens?.contains { $0.name == screen } == true }
    /// Its pages: the asker's own scope is the page `user`.
    var scopes: [String] { (installed ?? []).map { $0.hasPrefix("user:") ? "user" : $0 } }
}

extension AppTile {
    /// `app`'s tile; `seed` names it when it's not known here.
    init(app: AppInfo?, seed: String, size: CGFloat) {
        self.init(icon: app?.icon, seed: app?.name ?? seed, color: app?.color, image: app?.image, imageKind: app?.imageKind, size: size)
    }
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

/// How a shell reaches a node: the Mac's own, or the network's address from a phone.
protocol NodeTransport: Sendable {
    /// Sends a request and returns the local API's JSON reply, or throws its error.
    func send(_ method: String, _ path: String, body: Data?, timeout: TimeInterval) async throws -> Data
    /// An app screen's page.
    func page(app: String, screen: String, params: String, scope: String?) async throws -> String
    /// The node's events, until the connection drops.
    func events() -> AsyncThrowingStream<NodeEvent, Error>
    /// What to say when nothing answers.
    var unreachable: String { get }
    /// Lets this app in as its person's device, if it needs to be, showing the code it's approved by meanwhile.
    func join(showing: @MainActor @Sendable (String) -> Void) async throws
    /// What an app's call answered last, if this transport keeps it: its JSON, and when (ms).
    func kept(app: String, op: String, input: Data) async -> (answer: Data, at: Int64)?
}

extension NodeTransport {
    func join(showing: @MainActor @Sendable (String) -> Void) async throws {}
    func kept(app: String, op: String, input: Data) async -> (answer: Data, at: Int64)? { nil }
}

/// What this machine's own programs send its node to say they are: the token in the node's home, which only this
/// machine's user can read (src/api.rs's `LOCAL_TOKEN`). Only a Mac has a node of its own.
enum LocalToken {
    static func add(to request: inout URLRequest) {
        #if os(macOS)
        let env = ProcessInfo.processInfo.environment
        let cli = Auth.cli
        let data = env["XDG_DATA_HOME"].map { URL(filePath: $0) } ?? FileManager.default.homeDirectoryForCurrentUser.appending(path: ".local/share")
        let home = env["\(cli.uppercased())_HOME"].map { URL(filePath: $0) } ?? data.appending(path: cli)
        if let token = try? String(contentsOf: home.appending(path: "local-token"), encoding: .utf8) {
            request.setValue(token.trimmingCharacters(in: .whitespacesAndNewlines), forHTTPHeaderField: "x-one-local-token")
        }
        #elseif targetEnvironment(simulator)
        // A simulator shares its Mac's files: NODE_URL's node at 127.0.0.1 is that Mac's, and so is its token.
        let env = ProcessInfo.processInfo.environment
        if let mac = env["SIMULATOR_HOST_HOME"],
           let token = try? String(contentsOf: URL(filePath: mac).appending(path: ".local/share/\(Auth.cli)/local-token"), encoding: .utf8) {
            request.setValue(token.trimmingCharacters(in: .whitespacesAndNewlines), forHTTPHeaderField: "x-one-local-token")
        }
        #endif
    }
}

/// Asking again while no node answers, as one restarting for an update doesn't for a moment: at once, then a quarter
/// second, doubling, up to five.
enum Retry {
    /// How long to wait before asking again the `misses`th time running (from 0), in seconds.
    static func wait(_ misses: Int) -> Double {
        misses == 0 ? 0 : min(5, 0.125 * Double(1 << min(misses, 6)))
    }

    /// Sends `request`, again for a few seconds while nothing answers or the gateway reaches no node, so nothing ran;
    /// never one that timed out, which may have.
    static func send(_ request: URLRequest) async throws -> (Data, URLResponse) {
        for misses in 0... {
            do {
                let (data, response) = try await URLSession.shared.data(for: request)
                guard misses < 5, [502, 503, 530].contains((response as? HTTPURLResponse)?.statusCode ?? 0) else { return (data, response) }
            } catch let error as URLError where misses < 5 && [.cannotConnectToHost, .networkConnectionLost, .cannotFindHost, .notConnectedToInternet].contains(error.code) {
            }
            try await Task.sleep(for: .seconds(wait(misses)))
        }
        fatalError()
    }
}

/// The node's HTTP API: this machine's own node, or the network's address; with `auth`, as the person signed in.
struct HTTPNode: NodeTransport {
    var base: URL
    var auth: Auth?

    private var here: Bool { base.host() == "127.0.0.1" }

    var unreachable: String {
        String(localized: here ? "It isn’t running on this Mac." : "Your devices aren’t answering.")
    }

    /// A request as the person signed in, if they are: a refused token is renewed once. To this Mac's own node, it's
    /// also one of this Mac's own programs.
    private func signed(_ request: URLRequest) async throws -> (URLRequest, String?) {
        var request = request
        if here { LocalToken.add(to: &request) }
        guard let auth else { return (request, nil) }
        let token = try await auth.token()
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        return (request, token)
    }

    func send(_ method: String, _ path: String, body: Data?, timeout: TimeInterval) async throws -> Data {
        var url = base.appending(path: path)
        // A GET's body goes in its query: `{ params }` becomes `?params=`.
        if method == "GET", let body, let fields = try? JSONSerialization.jsonObject(with: body) as? [String: Any] {
            url.append(queryItems: fields.compactMap { k, v in (v as? String).map { URLQueryItem(name: k, value: $0) } })
        }
        var plain = URLRequest(url: url, timeoutInterval: timeout)
        plain.httpMethod = method
        if let body, method != "GET" {
            plain.setValue("application/json", forHTTPHeaderField: "Content-Type")
            plain.httpBody = body
        }
        var (request, token) = try await signed(plain)
        var (data, response) = try await Retry.send(request)
        if let auth, (response as? HTTPURLResponse)?.statusCode == 401 {
            request.setValue("Bearer \(try await auth.token(stale: token))", forHTTPHeaderField: "Authorization")
            (data, response) = try await Retry.send(request)
        }
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        if status != 200 {
            struct Failure: Decodable { var error: String? }
            let error = (try? JSONDecoder().decode(Failure.self, from: data))?.error
            if auth != nil, status == 401 { throw Auth.SignedOut(message: error ?? String(localized: "Sign in again.")) }
            throw NodeError(error ?? (!here && status >= 500 ? String(localized: "Your devices aren’t answering.") : String(localized: "That didn’t work. Try again.")))
        }
        return data
    }

    func page(app: String, screen: String, params: String, scope: String?) async throws -> String {
        var query = ["params": params]
        query["scope"] = scope
        let html = try await send("GET", "v1/screens/\(app)/\(screen)", body: try JSONEncoder().encode(query), timeout: 10)
        return String(decoding: html, as: UTF8.self)
    }

    func events() -> AsyncThrowingStream<NodeEvent, Error> {
        let url = base.appending(path: "v1/events")
        return AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    var plain = URLRequest(url: url, timeoutInterval: 60)
                    plain.setValue("text/event-stream", forHTTPHeaderField: "Accept")
                    let (request, token) = try await signed(plain)
                    let (bytes, response) = try await URLSession.shared.bytes(for: request)
                    if let auth, (response as? HTTPURLResponse)?.statusCode == 401 { _ = try await auth.token(stale: token) }
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
    /// The scope's owner: a user's name, a group's, or the network's.
    var scope_name: String
    var app: String
    var title: String?
    var fields: [Field]?
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
    var label: String { title ?? "" }
    var wanted: [Field] { fields ?? [] }
}

/// Backups, as the node says how they're going.
enum Backups {
    /// What's wrong with a place, and its fix, in words; since when.
    struct Problem: Decodable, Sendable, Equatable, Hashable { var place: String; var text: String; var fix: String; var since: Double }
}

/// The local API, typed, over whichever transport reaches a node.
struct NodeClient: Sendable {
    var transport: any NodeTransport
    /// What the node credits this shell's calls to, for apps' insights; a phone's device labels it itself.
    var client: String?

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

    /// The node's state and apps, each with the answer as it came, for the window to keep (`KeptWindow`).
    func state() async throws -> (NodeState, Data) { try await answer("v1/state", timeout: 2) }
    func apps() async throws -> ([AppInfo], Data) { try await answer("v1/apps") }

    private func answer<T: Decodable>(_ path: String, timeout: TimeInterval = 10) async throws -> (T, Data) {
        let data = try await transport.send("GET", path, body: nil, timeout: timeout)
        return (try JSONDecoder().decode(T.self, from: data), data)
    }

    /// The widgets, rendered for a page's scope.
    func widgets(scope: String?) async throws -> [WidgetInfo] {
        try await get("v1/widgets", scope.map { ["scope": $0] } ?? [:])
    }

    /// Each app's badge on a page's scope: the count its icon shows. A node too old to know badges has none.
    func badges(scope: String?) async throws -> [String: Int] {
        (try? await get("v1/badges", scope.map { ["scope": $0] } ?? [:])) ?? [:]
    }

    /// Runs an app's tool for an app screen: its input and result are JSON, whatever their shape.
    func call(app: String, op: String, input: Data, timeout: TimeInterval = 15) async throws -> Data {
        let input = try JSONSerialization.jsonObject(with: input, options: .fragmentsAllowed)
        // Its id makes it run once, however often it's sent.
        let data = try await transport.send("POST", "v1/call", body: try json(["id": Self.callID(), "app": app, "op": op, "input": input, "client": client]), timeout: timeout)
        let reply = try JSONSerialization.jsonObject(with: data) as? [String: Any] ?? [:]
        if let error = reply["error"] as? String { throw NodeError(error) }
        return try JSONSerialization.data(withJSONObject: reply["ok"] ?? NSNull(), options: .fragmentsAllowed)
    }

    /// What an app screen's call answered last, if its tool only reads: `{ answer, at }`, for bridge.js's `kept`.
    func kept(app: String, op: String, input: Data) async -> [String: Any]? {
        guard let kept = await transport.kept(app: app, op: op, input: input),
              let answer = try? JSONSerialization.jsonObject(with: kept.answer, options: .fragmentsAllowed) else { return nil }
        return ["answer": answer, "at": kept.at]
    }

    /// Names a call wherever it's sent, so sent again it runs once.
    static func callID() -> String {
        (0..<8).map { _ in String(format: "%02x", UInt8.random(in: 0...255)) }.joined()
    }

    func page(app: String, screen: String, params: String, scope: String?) async throws -> String {
        try await transport.page(app: app, screen: screen, params: params, scope: scope)
    }

    /// A Settings screen's `one.api`: `path` is the node's /v1 API's, its query's fields going in the body as every
    /// request's do (a device that a phone asks matches bare paths). → its JSON reply.
    func api(_ method: String, _ path: String, body: Any?) async throws -> Any {
        var parts = URLComponents(string: path)
        guard let bare = parts?.path, bare.hasPrefix("/v1/"), !bare.contains("..") else { throw NodeError("That isn’t on your devices.") }
        var fields = body as? [String: Any] ?? [:]
        for item in parts?.queryItems ?? [] { fields[item.name] = item.value ?? "" }
        if bare == "/v1/call", fields["id"] == nil { fields["id"] = Self.callID() }
        parts?.queryItems = nil
        let method = method.uppercased()
        let sent: Any? = method == "GET" ? (fields.isEmpty ? nil : fields) : (body is [String: Any] || body == nil ? fields : body)
        let data = try sent.map { try JSONSerialization.data(withJSONObject: $0, options: .fragmentsAllowed) }
        let reply = try JSONSerialization.jsonObject(with: try await transport.send(method, String(bare.dropFirst()), body: data, timeout: 30), options: .fragmentsAllowed)
        if let error = (reply as? [String: Any])?["error"] as? String { throw NodeError(error) }
        return reply
    }

    func events() -> AsyncThrowingStream<NodeEvent, Error> { transport.events() }

    // MARK: The network

    /// A pairing code for another of this user's computers.
    func pair() async throws -> String {
        try await post("v1/pair", [:])["code"] as? String ?? ""
    }

    func join(_ code: String) async throws { try await post("v1/join", ["code": code], timeout: 60) }

    func rename(_ name: String) async throws { try await post("v1/name", ["name": name]) }

    // MARK: Apps and the Vault

    /// The Vault's items on a page this user manages, or on all of them.
    func vault(scope: String? = nil) async throws -> [VaultItem] { try await get("v1/vault", scope.map { ["scope": $0] } ?? [:]) }

    /// Makes an item (no `id`) or changes one: fields as `{ name: { value, secret } }`, `apps`, `hosts`, `deleted`.
    @discardableResult
    func setItem(scope: String, id: String?, _ fields: [String: Any?]) async throws -> String? {
        var body = fields
        body["scope"] = scope
        if let id { body["id"] = id }
        return try await post("v1/vault", body)["id"] as? String
    }

    /// Asks this user for an item an app's need names, for an app screen's `one.vault(need)` → the request's id, and
    /// the scope it's in (their own, where they can't change the page's).
    func requestItem(scope: String, app: String, need: String) async throws -> (id: String, scope: String)? {
        let out = try await post("v1/vault/request", ["scope": scope, "app": app, "need": need])
        guard let id = out["id"] as? String, let scope = out["scope"] as? String else { return nil }
        return (id, scope)
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

}

/// A failed join, said as what went wrong and what to do next.
func joinFailure(_ error: Error) -> String {
    let lower = NodeError(error.localizedDescription).message.lowercased()
    if lower.contains("already used") || lower.contains("expired") || lower.contains("refused") {
        return String(localized: "That pairing code was used or has expired. Make a new one on your other device.")
    } else if lower.contains("already belongs") {
        let name = appName.hasPrefix("The ") ? String(appName.dropFirst(4)) : appName
        return String(localized: "This device is already in another \(name). Remove it there first.")
    } else if lower.contains("decode") || lower.contains("invalid") || lower.contains("base") || lower.contains("parse") || lower.contains("bad") {
        return String(localized: "That isn’t a whole pairing code. Copy it again, all of it.")
    } else if lower.contains("timed out") || lower.contains("timeout") || lower.contains("connect") || lower.contains("unreachable") || lower.contains("no route") || lower.contains("network") {
        return String(localized: "Couldn’t reach the device that made the code. Check both are online, then try again.")
    } else {
        return String(localized: "Couldn’t join. Make a new pairing code on your other device and try again.")
    }
}

/// The window as it was last, kept on this device until its person signs out: the node's state and apps as it last
/// gave them, and each page's widgets. The app draws it as it opens, while it asks the node again.
enum KeptWindow {
    private static var dir: URL { FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0].appending(path: "window") }

    static func keep(_ name: String, _ data: Data) {
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try? data.write(to: dir.appending(path: name), options: .atomic)
    }

    static func read<T: Decodable>(_ name: String, as: T.Type) -> T? {
        (try? Data(contentsOf: dir.appending(path: name))).flatMap { try? JSONDecoder().decode(T.self, from: $0) }
    }

    static func forget() { try? FileManager.default.removeItem(at: dir) }
}
