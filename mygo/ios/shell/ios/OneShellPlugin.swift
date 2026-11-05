import AuthenticationServices
import UIKit

/// The One App's shell on iPhone, beside MyGo's host: the system's sign-in sheet and menus.
final class OneShellPlugin: NSObject, MyGoPlugin, ASWebAuthenticationPresentationContextProviding {
    private var session: ASWebAuthenticationSession?

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

    func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        PluginHost.window ?? ASPresentationAnchor()
    }
}
