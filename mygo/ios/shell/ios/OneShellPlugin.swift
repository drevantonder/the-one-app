import AuthenticationServices
import UIKit
import UserNotifications

/// The One App's shell on iPhone, beside MyGo's host: the system's sign-in sheet and menus.
final class OneShellPlugin: NSObject, MyGoPlugin, ASWebAuthenticationPresentationContextProviding, ASAuthorizationControllerDelegate,
    ASAuthorizationControllerPresentationContextProviding {
    private var session: ASWebAuthenticationSession?
    private var passkeyReply: PluginReply?

    func call(_ method: String, _ args: [String: Any], _ reply: @escaping PluginReply) {
        switch method {
        case "openSignIn":
            guard let url = (args["url"] as? String).flatMap(URL.init(string:)) else {
                return reply(.failure(PluginFailure("BadArgs", "openSignIn takes url")))
            }
            // Sign-in comes back to the app's loopback port, which the sheet loads: no scheme of its own to end it,
            // so closeSignIn does. No "wants to use … to sign in" alert (which no one taps in a simulator), and
            // no cookies kept, as each sign-in asks for the passkey anyway.
            let session = ASWebAuthenticationSession(url: url, callbackURLScheme: "one-unused") { _, _ in }
            session.presentationContextProvider = self
            session.prefersEphemeralWebBrowserSession = true
            self.session = session
            session.start()
            reply(.success(nil))
        case "closeSignIn":
            session?.cancel()
            session = nil
            reply(.success(nil))
        case "group":
            // The App Group the app shares with its extensions: it leaves `text` there for them.
            guard let shared = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: args["group"] as? String ?? "") else {
                return reply(.failure(PluginFailure("NoGroup", "this app isn't in that App Group")))
            }
            do {
                try (args["text"] as? String ?? "").write(to: shared.appending(path: "from-app.txt"), atomically: true, encoding: .utf8)
                reply(.success(["path": shared.path]))
            } catch {
                reply(.failure(PluginFailure("Write", error.localizedDescription)))
            }
        case "notify":
            // Not provisional: the system runs a notification service extension only for notifications that alert.
            UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .badge, .sound]) { ok, error in
                DispatchQueue.main.async { UIApplication.shared.registerForRemoteNotifications() }
                reply(.success(["granted": ok]))
            }
        case "passkey":
            // A passkey assertion for `rp` (the install's sign-in), as apple/Shared signs in natively: the system's
            // passkey sheet, which needs the app associated with the domain (webcredentials, its AASA file).
            guard let rp = args["rp"] as? String, let challenge = (args["challenge"] as? String).flatMap({ Data(base64Encoded: $0) }) else {
                return reply(.failure(PluginFailure("BadArgs", "passkey takes rp and challenge")))
            }
            let request = ASAuthorizationPlatformPublicKeyCredentialProvider(relyingPartyIdentifier: rp).createCredentialAssertionRequest(challenge: challenge)
            let controller = ASAuthorizationController(authorizationRequests: [request])
            controller.delegate = self
            controller.presentationContextProvider = self
            passkeyReply = reply
            controller.performRequests()
        case "menu":
            let items = args["items"] as? [[String: Any]] ?? []
            let sheet = UIAlertController(title: nil, message: nil, preferredStyle: .actionSheet)
            for (i, item) in items.enumerated() {
                let title = (item["checked"] as? Bool == true ? "✓ " : "") + (item["title"] as? String ?? "")
                sheet.addAction(UIAlertAction(title: title, style: item["danger"] as? Bool == true ? .destructive : .default) { _ in reply(.success(["index": i])) })
            }
            sheet.addAction(UIAlertAction(title: "Cancel", style: .cancel) { _ in reply(.success(["index": NSNull()])) })
            if let pop = sheet.popoverPresentationController, let view = PluginHost.presenter.view {
                pop.sourceView = view
                pop.sourceRect = CGRect(x: view.bounds.midX, y: view.bounds.midY, width: 1, height: 1)
            }
            PluginHost.present(sheet)
        default:
            reply(.failure(PluginFailure("NoMethod", method)))
        }
    }

    func authorizationController(controller: ASAuthorizationController, didCompleteWithAuthorization authorization: ASAuthorization) {
        let assertion = authorization.credential as? ASAuthorizationPlatformPublicKeyCredentialAssertion
        passkeyReply?(.success(["credential": assertion?.credentialID.base64EncodedString() ?? "", "signature": assertion?.signature.base64EncodedString() ?? ""]))
        passkeyReply = nil
    }

    func authorizationController(controller: ASAuthorizationController, didCompleteWithError error: Error) {
        let e = error as NSError
        passkeyReply?(.failure(PluginFailure("Passkey", "\(e.domain) \(e.code): \(e.localizedDescription) \(e.userInfo[NSLocalizedFailureReasonErrorKey] ?? "")")))
        passkeyReply = nil
    }

    func presentationAnchor(for controller: ASAuthorizationController) -> ASPresentationAnchor {
        PluginHost.window ?? ASPresentationAnchor()
    }

    func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        PluginHost.window ?? ASPresentationAnchor()
    }
}
