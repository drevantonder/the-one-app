import Foundation
import AuthenticationServices
import CryptoKit
import Security

/// Signing in, as every client of the install's auth service does (kernel/web/web.js is the same in a browser): this
/// app registers itself as an OAuth client, the person signs in with their passkey, and it comes back with a code that
/// PKCE turns into tokens. On the iPhone that's the system's sign-in sheet, back at `<cli>://signed-in/`; on the Mac,
/// the person's own default browser, where their passkey is, back at a port on this Mac (`Loopback`). The refresh
/// token is kept in the Keychain, for this device only; access tokens only in memory.
actor Auth {
    static let shared = Auth()

    struct SignedOut: LocalizedError {
        var message: String
        var errorDescription: String? { message }
    }

    private static var defaults: UserDefaults { .standard }
    static let cli = Bundle.main.object(forInfoDictionaryKey: "CLI") as? String ?? "one"
    /// What this app registers as its redirect: the Mac's, a loopback address, for whatever port is free.
    private static let redirect: String = {
        #if os(macOS)
        return "http://127.0.0.1/callback"
        #else
        return "\(cli)://signed-in/"
        #endif
    }()

    /// The person cancelled signing in.
    struct Cancelled: Error {}

    private var access: String?
    private var until = Date.distantPast
    /// Where the refresh token is: kept and unused, being swapped for a new one (each works once, so one swap at a
    /// time), or gone, signed out, until the person signs in again.
    private enum State { case idle, refreshing(Task<String, Error>), signedOut }
    private var state = State.idle

    /// Where the install is: the one the person gave.
    nonisolated static var address: URL? {
        get { defaults.string(forKey: "address").flatMap(URL.init(string:)) }
        set {
            if newValue?.absoluteString != defaults.string(forKey: "address") {
                defaults.removeObject(forKey: "client")
                defaults.removeObject(forKey: "clientRedirect")
            }
            defaults.set(newValue?.absoluteString, forKey: "address")
        }
    }

    nonisolated static var signedIn: Bool {
        #if DEBUG
        takeTestSignIn()
        #endif
        return address != nil && Keychain.read() != nil
    }

    #if DEBUG
    /// A debug build is signed in by `SIGN_IN` (`{ address, client, refresh }`, a sign-in made elsewhere), as `one try`
    /// tests one where no one can tap the system's sign-in sheet: in the simulator, which takes no taps from agents.
    nonisolated private static func takeTestSignIn() {
        guard let given = ProcessInfo.processInfo.environment["SIGN_IN"], defaults.string(forKey: "testSignIn") != given,
              let fields = try? JSONSerialization.jsonObject(with: Data(given.utf8)) as? [String: String],
              let at = fields["address"].flatMap(URL.init(string:)), let client = fields["client"], let refresh = fields["refresh"] else { return }
        address = at
        defaults.set(client, forKey: "client")
        defaults.set(given, forKey: "testSignIn")
        Keychain.write(refresh)
    }
    #endif

    /// The secret of the invite link this person opened here, for their first device's request to be let in.
    nonisolated static var invite: String? {
        get { defaults.string(forKey: "invite") }
        set { defaults.set(newValue, forKey: "invite") }
    }

    /// Takes what the person typed: an address, or an invite link (`<address>/invite#…`). → the invite link, if it
    /// was one.
    nonisolated static func take(_ typed: String) throws -> URL? {
        var text = typed.trimmingCharacters(in: .whitespacesAndNewlines)
        if !text.contains("://") { text = "https://" + text }
        guard let url = URL(string: text), let host = url.host(), url.scheme == "https" || (url.scheme == "http" && ["localhost", "127.0.0.1"].contains(host)) else {
            throw SignedOut(message: String(localized: "That isn’t an address. It looks like one.example.com."))
        }
        var origin = URLComponents()
        origin.scheme = url.scheme
        origin.host = host
        origin.port = url.port
        address = origin.url
        guard url.path() == "/invite", let secret = url.fragment(), !secret.isEmpty else { return nil }
        invite = secret
        return url
    }

    private struct Meta: Decodable {
        var authorization_endpoint: String
        var token_endpoint: String
        var registration_endpoint: String
        var revocation_endpoint: String?
    }

    private func meta() async throws -> Meta {
        guard let address = Self.address else { throw SignedOut(message: String(localized: "Enter the address first.")) }
        do {
            let (data, response) = try await URLSession.shared.data(from: address.appending(path: ".well-known/oauth-authorization-server"))
            guard (response as? HTTPURLResponse)?.statusCode == 200 else { throw URLError(.badServerResponse) }
            return try JSONDecoder().decode(Meta.self, from: data)
        } catch {
            throw NodeError(String(localized: "Can’t reach that address. Check it, and that you’re online."))
        }
    }

    /// This app, as a client of the auth service: registered the first time it signs in at an address.
    private func client(_ meta: Meta, name: String) async throws -> String {
        if let id = Self.defaults.string(forKey: "client"), Self.defaults.string(forKey: "clientRedirect") == Self.redirect { return id }
        var request = URLRequest(url: URL(string: meta.registration_endpoint)!)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONSerialization.data(withJSONObject: [
            "client_name": name, "redirect_uris": [Self.redirect], "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"], "response_types": ["code"], "application_type": "native",
        ])
        let (data, response) = try await URLSession.shared.data(for: request)
        let out = (try? JSONSerialization.jsonObject(with: data) as? [String: Any]) ?? [:]
        guard (200..<300).contains((response as? HTTPURLResponse)?.statusCode ?? 0), let id = out["client_id"] as? String else {
            throw NodeError(out["error_description"] as? String ?? out["message"] as? String ?? String(localized: "This device couldn’t sign up to sign in."))
        }
        Self.defaults.set(id, forKey: "client")
        Self.defaults.set(Self.redirect, forKey: "clientRedirect")
        return id
    }

    /// Signs in (`authenticate` opens a URL for the person to sign in at, and gives where it came back to; it throws
    /// `Cancelled` if they gave up), calling this device `name` among the person's signed-in devices, and coming back
    /// to `redirect` if it isn't the one registered (a loopback address's port). It names the device it's for (`join`),
    /// as every client's sign-in does. False if they cancelled.
    func signIn(name: String, redirect: String? = nil, authenticate: @MainActor @Sendable (URL) async throws -> URL) async throws -> Bool {
        let redirect = redirect ?? Self.redirect
        let meta = try await meta()
        let client = try await client(meta, name: name)
        let verifier = Self.random(32), state = Self.random(16)
        var url = URLComponents(string: meta.authorization_endpoint)!
        url.queryItems = [
            .init(name: "response_type", value: "code"), .init(name: "client_id", value: client), .init(name: "redirect_uri", value: redirect),
            .init(name: "scope", value: "openid profile offline_access"), .init(name: "code_challenge", value: Self.b64u(Data(SHA256.hash(data: Data(verifier.utf8))))),
            .init(name: "code_challenge_method", value: "S256"), .init(name: "state", value: state), .init(name: "prompt", value: "consent"),
        ]
        // The device it's for, so the person's devices can let it in by this sign-in.
        if let join = DeviceKeys.signingInFor() { url.queryItems?.append(.init(name: "join", value: join)) }
        let back: URL
        do {
            back = try await authenticate(url.url!)
        } catch let error as ASWebAuthenticationSessionError where error.code == .canceledLogin {
            return false
        } catch is Cancelled {
            return false
        }
        let query = URLComponents(url: back, resolvingAgainstBaseURL: false)?.queryItems ?? []
        let value = { (name: String) in query.first { $0.name == name }?.value }
        if let error = value("error") {
            throw NodeError(error == "access_denied" ? String(localized: "You didn’t let this device in.") : value("error_description") ?? error)
        }
        guard value("state") == state, let code = value("code") else { throw NodeError(String(localized: "That sign-in didn’t start here. Sign in again.")) }
        try await grant(meta, ["grant_type": "authorization_code", "code": code, "redirect_uri": redirect, "client_id": client, "code_verifier": verifier])
        self.state = .idle
        return true
    }

    /// Swaps a code or refresh token for tokens: each refresh token works once, so the new one is kept first.
    private func grant(_ meta: Meta, _ fields: [String: String]) async throws {
        var request = URLRequest(url: URL(string: meta.token_endpoint)!)
        request.httpMethod = "POST"
        request.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
        request.httpBody = Data(Self.form(fields).utf8)
        let (data, response) = try await URLSession.shared.data(for: request)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        let out = (try? JSONSerialization.jsonObject(with: data) as? [String: Any]) ?? [:]
        guard status == 200, let token = out["access_token"] as? String else {
            let why = out["error_description"] as? String ?? out["error"] as? String ?? String(localized: "Signing in didn’t work.")
            if status == 400 || status == 401 { throw SignedOut(message: why) }
            throw NodeError(why)
        }
        if let refresh = out["refresh_token"] as? String { Keychain.write(refresh) }
        access = token
        until = Date().addingTimeInterval(out["expires_in"] as? Double ?? 3600)
    }

    /// An access token good for a minute more, new from the refresh token if it's older, or if the node refused
    /// `stale`: one refresh at a time, since each refresh token works once.
    func token(stale: String? = nil) async throws -> String {
        if let access, access != stale, until > Date().addingTimeInterval(60) { return access }
        switch state {
        case .signedOut: throw SignedOut(message: String(localized: "Sign in again."))
        case .refreshing(let task): return try await task.value
        case .idle: break
        }
        let task = Task<String, Error> {
            guard let refresh = Keychain.read() else { throw SignedOut(message: String(localized: "Sign in again.")) }
            let meta = try await meta()
            let fields = ["grant_type": "refresh_token", "refresh_token": refresh, "client_id": Self.defaults.string(forKey: "client") ?? ""]
            // An answer lost on the way back leaves the token this one sent, which the auth service still takes for a
            // moment: so it's sent again at once, and only a refusal signs this device out.
            var attempt = 0
            while true {
                do {
                    try await grant(meta, fields)
                    return access ?? ""
                } catch let error as SignedOut {
                    Keychain.write(nil)
                    access = nil
                    state = .signedOut
                    throw error
                } catch is URLError where attempt < 3 {
                    try await Task.sleep(for: .milliseconds(500 << attempt))
                    attempt += 1
                    // Signing out meanwhile: what it revokes is this token, not one a later answer gives.
                    if case .signedOut = state { throw SignedOut(message: String(localized: "Sign in again.")) }
                }
            }
        }
        state = .refreshing(task)
        defer { if case .refreshing = state { state = .idle } }
        return try await task.value
    }

    /// Signs out: the refresh token is revoked, so it opens nothing again, and forgotten here. A refresh on its way
    /// finishes first, so what it kept is what's revoked and forgotten, not kept again after.
    func signOut() async {
        if case .refreshing(let task) = state {
            state = .signedOut
            _ = try? await task.value
        }
        state = .signedOut
        let refresh = Keychain.read()
        Keychain.write(nil)
        access = nil
        guard let refresh, let meta = try? await meta(), let revoke = meta.revocation_endpoint.flatMap(URL.init(string:)) else { return }
        var request = URLRequest(url: revoke)
        request.httpMethod = "POST"
        request.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
        request.httpBody = Data(Self.form(["token": refresh, "token_type_hint": "refresh_token", "client_id": Self.defaults.string(forKey: "client") ?? ""]).utf8)
        _ = try? await URLSession.shared.data(for: request)
    }

    private static func form(_ fields: [String: String]) -> String {
        var allowed = CharacterSet.alphanumerics
        allowed.insert(charactersIn: "-._~")
        return fields.map { "\($0.key.addingPercentEncoding(withAllowedCharacters: allowed)!)=\($0.value.addingPercentEncoding(withAllowedCharacters: allowed)!)" }.joined(separator: "&")
    }

    private static func b64u(_ data: Data) -> String {
        data.base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: "=", with: "")
    }

    private static func random(_ n: Int) -> String {
        var bytes = [UInt8](repeating: 0, count: n)
        _ = SecRandomCopyBytes(kSecRandomDefault, n, &bytes)
        return b64u(Data(bytes))
    }

    /// The refresh token, in the Keychain: on this device only, readable once it has been unlocked since it started,
    /// so a widget's button works from the lock screen. The Mac's is in the login keychain, which only this app reads.
    private enum Keychain {
        static var query: [String: Any] { [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: "\(cli) sign-in", kSecAttrAccount as String: "refresh"] }

        static func read() -> String? {
            var out: CFTypeRef?
            guard SecItemCopyMatching(query.merging([kSecReturnData as String: true]) { $1 } as CFDictionary, &out) == errSecSuccess, let data = out as? Data else { return nil }
            return String(decoding: data, as: UTF8.self)
        }

        static func write(_ value: String?) {
            SecItemDelete(query as CFDictionary)
            guard let value else { return }
            var item = query.merging([kSecValueData as String: Data(value.utf8)]) { $1 }
            #if os(iOS)
            item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            #endif
            SecItemAdd(item as CFDictionary, nil)
        }
    }
}
