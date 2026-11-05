import AuthenticationServices
import ObjectiveC
import SwiftRs
import Tauri
import UIKit
import UserNotifications
import WebKit

struct MenuArgs: Decodable {
  struct Item: Decodable {
    let title: String
    let danger: Bool?
  }
  let items: [Item]
}

struct PasskeyArgs: Decodable {
  let rpId: String
}

struct GroupArgs: Decodable {
  let group: String
  let write: String?
}

/// The shell's natives on an iPhone: what Tauri's own plugins don't do.
class OneNativesPlugin: Plugin {
  var passkey: PasskeyDelegate?
  static var tokenWaiter: ((String?, String?) -> Void)?

  /// A screen's `one.menu`, as an action sheet: a danger item is destructive (red), as the Apple app's are.
  @objc public func menu(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(MenuArgs.self)
    DispatchQueue.main.async {
      let sheet = UIAlertController(title: nil, message: nil, preferredStyle: .actionSheet)
      for (i, item) in args.items.enumerated() {
        sheet.addAction(UIAlertAction(title: item.title, style: item.danger == true ? .destructive : .default) { _ in invoke.resolve(["index": i]) })
      }
      sheet.addAction(UIAlertAction(title: "Cancel", style: .cancel) { _ in invoke.resolve(["index": -1]) })
      if let pop = sheet.popoverPresentationController, let view = self.manager.viewController?.view {
        pop.sourceView = view
        pop.sourceRect = CGRect(x: view.bounds.midX, y: view.bounds.midY, width: 1, height: 1)
      }
      self.manager.viewController?.present(sheet, animated: true)
    }
  }

  /// A passkey assertion for `rpId`, as the sign-in page would ask: the plugin side only (the install serves no
  /// apple-app-site-association, so iOS refuses the domain).
  @objc public func passkey(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(PasskeyArgs.self)
    DispatchQueue.main.async {
      let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(relyingPartyIdentifier: args.rpId)
      let challenge = Data((0..<32).map { _ in UInt8.random(in: 0...255) })
      let controller = ASAuthorizationController(authorizationRequests: [provider.createCredentialAssertionRequest(challenge: challenge)])
      let delegate = PasskeyDelegate(invoke: invoke, anchor: self.manager.viewController?.view.window ?? UIWindow())
      self.passkey = delegate
      controller.delegate = delegate
      controller.presentationContextProvider = delegate
      controller.performRequests()
    }
  }

  /// Registers for remote notifications → the APNs token (hex), and asks for provisional authorization, which shows no
  /// prompt. Tauri's app delegate has no push callbacks, so they're added to its class here.
  @objc public func push(_ invoke: Invoke) throws {
    DispatchQueue.main.async {
      Self.addTokenCallbacks()
      UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .badge, .sound, .provisional]) { granted, error in
        UNUserNotificationCenter.current().getNotificationSettings { settings in
          let auth = settings.authorizationStatus.rawValue
          Self.tokenWaiter = { token, failure in
            Self.tokenWaiter = nil
            invoke.resolve(["token": token as Any, "error": failure as Any, "provisional": granted, "authorization": auth, "authError": error.map { "\($0)" } as Any])
          }
          DispatchQueue.main.async { UIApplication.shared.registerForRemoteNotifications() }
        }
      }
    }
  }

  /// The App Group's folder: reads what the notification extension left there (`opened.json`), and writes `write` to
  /// `from-app.txt`.
  @objc public func group(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(GroupArgs.self)
    guard let dir = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: args.group) else {
      return invoke.reject("No App Group \(args.group): the app's entitlements don't name it.")
    }
    if let text = args.write { try? text.write(to: dir.appendingPathComponent("from-app.txt"), atomically: true, encoding: .utf8) }
    let opened = try? String(contentsOf: dir.appendingPathComponent("opened.json"), encoding: .utf8)
    invoke.resolve(["path": dir.path, "opened": opened as Any])
  }

  static func addTokenCallbacks() {
    guard let delegate = UIApplication.shared.delegate else { return }
    let cls: AnyClass = type(of: delegate)
    let ok: @convention(block) (AnyObject, UIApplication, Data) -> Void = { _, _, token in
      tokenWaiter?(token.map { String(format: "%02x", $0) }.joined(), nil)
    }
    let fail: @convention(block) (AnyObject, UIApplication, NSError) -> Void = { _, _, error in
      tokenWaiter?(nil, "\(error.domain) \(error.code): \(error.localizedDescription)")
    }
    for (sel, block) in [
      (#selector(UIApplicationDelegate.application(_:didRegisterForRemoteNotificationsWithDeviceToken:)), ok as Any),
      (#selector(UIApplicationDelegate.application(_:didFailToRegisterForRemoteNotificationsWithError:)), fail as Any),
    ] {
      let imp = imp_implementationWithBlock(block)
      if !class_addMethod(cls, sel, imp, "v@:@@"), let m = class_getInstanceMethod(cls, sel) {
        method_setImplementation(m, imp)
      }
    }
  }
}

class PasskeyDelegate: NSObject, ASAuthorizationControllerDelegate, ASAuthorizationControllerPresentationContextProviding {
  let invoke: Invoke
  let anchor: UIWindow
  init(invoke: Invoke, anchor: UIWindow) {
    self.invoke = invoke
    self.anchor = anchor
  }
  func presentationAnchor(for controller: ASAuthorizationController) -> ASPresentationAnchor { anchor }
  func authorizationController(controller: ASAuthorizationController, didCompleteWithAuthorization authorization: ASAuthorization) {
    invoke.resolve(["ok": true])
  }
  func authorizationController(controller: ASAuthorizationController, didCompleteWithError error: Error) {
    let e = error as NSError
    invoke.resolve(["ok": false, "domain": e.domain, "code": e.code, "message": e.localizedDescription])
  }
}

@_cdecl("init_plugin_one_natives")
func initPlugin() -> Plugin {
  return OneNativesPlugin()
}
