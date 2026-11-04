import CryptoKit
import Foundation
import Security

/// This app as a device of its person (client/ffi, the library every native app shares): its own key, made here and
/// kept in the Keychain, one for each person at each address. It gets in when one of their devices approves the code it
/// shows; then every request reaches their install's nodes over iroh, as this device, with the keys each needs.
/// Signing in only says who's asking.
actor DeviceClient {
    static let shared = DeviceClient()

    private var device: ThisDevice?
    private var deviceFor: String?

    /// This app's device for whoever's signed in, at their address.
    private func current() async throws -> ThisDevice {
        let token = try await Auth.shared.token()
        guard let address = Auth.address?.absoluteString else { throw Auth.SignedOut(message: String(localized: "Sign in again.")) }
        let id = "\(address) \(Self.subject(token) ?? "")"
        if let device, deviceFor == id { return device }
        let folder = URL.applicationSupportDirectory.appending(path: "client/\(Keychain.name(id))")
        let made = try await off { try ThisDevice(secret: Keychain.secret(id), address: address, dir: folder.path()) }
        if let on = await Self.on() { made.on(device: on) }
        if let old = device { await off { old.disconnect() } }
        device = made
        deviceFor = id
        return made
    }

    /// The device this app is on, which signing in here again replaces its sign-in under: on a Mac, its oned's key
    /// (the Mac as a device); on an iPhone, an id of its own, hashed, kept in this device's Keychain, which outlives
    /// signing out.
    private static func on() async -> String? {
        #if os(macOS)
        let port = Bundle.main.object(forInfoDictionaryKey: "NodePort") as? String ?? ""
        guard let url = URL(string: "http://127.0.0.1:\(port)/v1/state") else { return nil }
        var request = URLRequest(url: url)
        LocalToken.add(to: &request)
        guard let (data, _) = try? await URLSession.shared.data(for: request),
              let state = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let device = state["device"] as? [String: Any] else { return nil }
        return device["key"] as? String
        #else
        return SHA256.hash(data: Keychain.secret("this device")).map { String(format: "%02x", $0) }.joined()
        #endif
    }

    /// Whether it's been let in: it reaches a node as itself.
    func connect() async throws -> Bool {
        let device = try await current()
        return try await off { try device.connect() }
    }

    /// Asks to be let in, as the person signed in, calling it `name` → the code it shows.
    func ask(name: String) async throws -> String {
        let device = try await current()
        let token = try await Auth.shared.token()
        try await off { try device.ask(token: token, name: name, invite: Auth.invite) }
        return device.code()
    }

    /// Whether it's been let in, asking again if its request waited too long.
    func waited() async throws -> Bool {
        let device = try await current()
        let token = try await Auth.shared.token()
        return try await off { try device.wait(token: token) }
    }

    /// A request to the person's nodes' API (`/v1/…`). Once this device isn't one of the install's, `lost`.
    func fetch(_ method: String, _ path: String, body: Data?, accept: String? = nil) async throws -> Response {
        let device = try await current()
        let headers = [body.map { _ in Header(name: "content-type", value: "application/json") }, accept.map { Header(name: "accept", value: $0) }].compactMap { $0 }
        do {
            return try await off { try device.fetch(method: method, path: path, headers: headers, body: body ?? Data()) }
        } catch is NotLetIn {
            throw await lost(device)
        }
    }

    /// Developer mode, on this device alone (Settings › Developer): whether it's on, first turning it on or off.
    func developer(_ on: Bool?) async throws -> Bool {
        let device = try await current()
        return try await off {
            if let on { device.setDeveloper(on: on) }
            return device.developer()
        }
    }

    /// In developer mode, its latest requests' traces, as JSON, for a screen's `traces`; nil out of it.
    func traces() async throws -> String? {
        let device = try await current()
        return try await off { device.traces() }
    }

    /// What it means that this device isn't one of the install's: if it was, it's been taken out, which is the sign-in's
    /// to say, as it signs out; if it never was, it's waiting to be approved, which its approval screen says.
    func lost(_ known: ThisDevice? = nil) async -> Error {
        let device: ThisDevice
        if let known { device = known } else if let made = try? await current() { device = made } else { return NotLetIn() }
        guard (try? await off { device.removed() }) == true else { return NotLetIn() }
        if let deviceFor { Keychain.forget(deviceFor) }
        await close()
        return Auth.SignedOut(message: String(localized: "This device was taken out. Sign in again."))
    }

    /// Signs out: this device takes itself out of its person's devices (it asks their node to), then its key goes, so
    /// signing in again makes a new device that replaces it, never a second. Asked before the sign-in is revoked.
    func signOut() async {
        if let device { await off { device.signOut() } }
        if let deviceFor { Keychain.forget(deviceFor) }
        await close()
    }

    /// What an app's call answered last for `input`, if its operation only reads (client/ffi's `kept`), from this
    /// device alone: its answer's JSON, and when (ms).
    func kept(app: String, op: String, input: Data) async -> (answer: Data, at: Int64)? {
        let device: ThisDevice
        if let made = self.device { device = made } else if let made = try? await current() { device = made } else { return nil }
        return (try? await off { device.kept(app: app, op: op, input: input).map { ($0.answer, $0.at) } }) ?? nil
    }

    /// Its connections close. Its key stays.
    func close() async {
        if let device { await off { device.disconnect() } }
        device = nil
        deviceFor = nil
    }

    /// Who an access token is for: its `sub`.
    static func subject(_ token: String) -> String? {
        let parts = token.split(separator: ".")
        guard parts.count > 1 else { return nil }
        var text = parts[1].replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        text += String(repeating: "=", count: (4 - text.count % 4) % 4)
        guard let data = Data(base64Encoded: text), let claims = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }
        return claims["sub"] as? String
    }

    /// The device keys, in the Keychain: on this device only, readable once it has been unlocked since it started.
    private enum Keychain {
        static func name(_ id: String) -> String {
            String(id.utf8.reduce(UInt64(14_695_981_039_346_656_037)) { ($0 ^ UInt64($1)) &* 1_099_511_628_211 }, radix: 16)
        }

        static func query(_ id: String) -> [String: Any] {
            [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: "\(Auth.cli) device", kSecAttrAccount as String: id]
        }

        /// `id`'s key goes: a key the install removed can't be let in again, so the next sign-in makes a new one.
        static func forget(_ id: String) {
            SecItemDelete(query(id) as CFDictionary)
        }

        /// `id`'s key, made the first time.
        static func secret(_ id: String) -> Data {
            var out: CFTypeRef?
            if SecItemCopyMatching(query(id).merging([kSecReturnData as String: true]) { $1 } as CFDictionary, &out) == errSecSuccess, let data = out as? Data, data.count == 32 {
                return data
            }
            let secret = newDeviceSecret()
            SecItemDelete(query(id) as CFDictionary)
            var item = query(id).merging([kSecValueData as String: secret]) { $1 }
            #if os(iOS)
            item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            #endif
            SecItemAdd(item as CFDictionary, nil)
            return secret
        }
    }
}

/// This device isn't one of the install's: not yet, or no more. Not an app's problem: the window shows its own
/// approval screen (`Store.Phase.joining`), or the sign-in.
struct NotLetIn: LocalizedError {
    var errorDescription: String? { String(localized: "This device hasn’t been let in yet.") }
}

/// Runs a call into the library, which waits for its answer, off the caller's thread; its failure in a person's words.
func off<T: Sendable>(_ work: @escaping @Sendable () throws -> T) async throws -> T {
    try await withCheckedThrowingContinuation { done in
        DispatchQueue.global(qos: .userInitiated).async {
            do {
                done.resume(returning: try work())
            } catch let OneError.Failed(reason, _) {
                done.resume(throwing: NodeError(reason))
            } catch OneError.NotLetIn {
                done.resume(throwing: NotLetIn())
            } catch {
                done.resume(throwing: error)
            }
        }
    }
}

func off(_ work: @escaping @Sendable () -> Void) async {
    await withCheckedContinuation { done in
        DispatchQueue.global(qos: .userInitiated).async {
            work()
            done.resume()
        }
    }
}

/// The person's nodes, reached over iroh as this app's own device.
struct DeviceNode: NodeTransport {
    var unreachable: String { String(localized: "Your devices aren’t answering.") }

    func send(_ method: String, _ path: String, body: Data?, timeout: TimeInterval) async throws -> Data {
        var parts = URLComponents()
        parts.path = "/" + path
        var sent = body
        // A GET's body goes in its query: `{ params }` becomes `?params=`.
        if method == "GET", let body, let fields = try? JSONSerialization.jsonObject(with: body) as? [String: Any] {
            parts.queryItems = fields.compactMap { k, v in (v as? String).map { URLQueryItem(name: k, value: $0) } }
            sent = nil
        }
        let request = parts.percentEncodedPath + (parts.percentEncodedQuery.map { "?" + $0.replacingOccurrences(of: "+", with: "%2B") } ?? "")
        let response = try await DeviceClient.shared.fetch(method, request, body: sent)
        let data = try await off { try response.bytes() }
        let status = response.status()
        if status != 200 {
            struct Failure: Decodable { var error: String? }
            let error = (try? JSONDecoder().decode(Failure.self, from: data))?.error
            throw NodeError(error ?? (status >= 500 ? String(localized: "Your devices aren’t answering.") : String(localized: "That didn’t work. Try again.")))
        }
        return data
    }

    func kept(app: String, op: String, input: Data) async -> (answer: Data, at: Int64)? {
        await DeviceClient.shared.kept(app: app, op: op, input: input)
    }

    func page(app: String, screen: String, params: String, scope: String?) async throws -> String {
        var query = ["params": params]
        query["scope"] = scope
        let html = try await send("GET", "v1/screens/\(app)/\(screen)", body: try JSONEncoder().encode(query), timeout: 10)
        return String(decoding: html, as: UTF8.self)
    }

    func events() -> AsyncThrowingStream<NodeEvent, Error> {
        AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    let response = try await DeviceClient.shared.fetch("GET", "/v1/events", body: nil, accept: "text/event-stream")
                    defer { response.cancel() }
                    guard response.status() == 200 else { throw NodeError(String(localized: "Your devices aren’t answering.")) }
                    var rest = Data()
                    while !Task.isCancelled, let part = try await off({ try response.chunk() }) {
                        rest.append(part)
                        while let end = rest.firstIndex(of: UInt8(ascii: "\n")) {
                            let line = String(decoding: rest[rest.startIndex..<end], as: UTF8.self)
                            rest = Data(rest[rest.index(after: end)...])
                            guard line.hasPrefix("data:") else { continue }
                            let json = line.dropFirst(5).trimmingCharacters(in: .whitespaces)
                            if let event = try? JSONDecoder().decode(NodeEvent.self, from: Data(json.utf8)) { continuation.yield(event) }
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

    /// Lets this app in as its person's device: at once if it is, else it asks, shows its code, and waits for one of
    /// their devices (or, for someone new, their inviter's) to approve it; `approve`, if given, tries first.
    func join(name: String, showing: @MainActor @Sendable (String) -> Void, approve: (@Sendable (String) async -> Void)? = nil) async throws {
        if try await DeviceClient.shared.connect() { return }
        let lost = await DeviceClient.shared.lost()
        if lost is Auth.SignedOut { throw lost }
        let code = try await DeviceClient.shared.ask(name: name)
        await showing(code)
        await approve?(code)
        while try await !DeviceClient.shared.waited() {
            try await Task.sleep(for: .seconds(2))
        }
        Auth.invite = nil
    }
}
