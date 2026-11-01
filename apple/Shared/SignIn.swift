import SwiftUI
import AuthenticationServices

/// The config's own sign-in (config.toml's `sign_in` and `account`), for a context with one install: the first run offers
/// it instead of starting an install. It's the work-email join driver's page, which signs the person in with their
/// work account and ends at `<cli>://join?code=…&user=…`, a pairing code for this device and their name.
struct SignIn {
    var url: URL
    var account: String

    static let configured: SignIn? = {
        let info = Bundle.main.infoDictionary ?? [:]
        guard let text = info["SignIn"] as? String, !text.isEmpty, let url = URL(string: text) else { return nil }
        return SignIn(url: url, account: info["SignInAccount"] as? String ?? "")
    }()

    var title: String {
        account.isEmpty ? String(localized: "Sign In") : String(localized: "Sign in with your \(account) account")
    }

    /// Signs in, in the system's sign-in sheet: a pairing code for this device, and the person's name, or nil if
    /// they cancelled.
    func run(_ session: WebAuthenticationSession) async throws -> (code: String, name: String)? {
        let info = Bundle.main.infoDictionary ?? [:]
        let cli = info["CLI"] as? String ?? ""
        let start = url.appending(path: "start").appending(queryItems: [URLQueryItem(name: "app", value: "1")])
        let back: URL
        do {
            back = try await session.authenticate(using: start, callback: .customScheme(cli), additionalHeaderFields: [:])
        } catch let error as ASWebAuthenticationSessionError where error.code == .canceledLogin {
            return nil
        }
        let query = URLComponents(url: back, resolvingAgainstBaseURL: false)?.queryItems ?? []
        let value = { (name: String) in query.first { $0.name == name }?.value ?? "" }
        if !value("error").isEmpty { throw SignInError(message: value("error")) }
        guard !value("code").isEmpty else {
            throw SignInError(message: String(localized: "Signing in didn’t give this device a way in. Try again, or use a pairing code."))
        }
        return (value("code"), value("user"))
    }
}

struct SignInError: LocalizedError {
    var message: String
    var errorDescription: String? { message }
}
