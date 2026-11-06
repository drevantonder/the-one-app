import AuthenticationServices
import ObjectiveC
import Tauri
import UIKit
import UserNotifications
import WebKit
import WidgetKit

/// The app's Rust (src/platform/iphone.rs `one_ios`): what happened here, which it acts on → its answer. It waits, so it's
/// called off the main thread.
@_silgen_name("one_ios") func oneIOS(_ kind: UnsafePointer<CChar>, _ json: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?
@_silgen_name("one_ios_free") func oneIOSFree(_ s: UnsafeMutablePointer<CChar>?)

enum Rust {
  /// Tells the app's Rust `kind` with `value` → what it answers (`ok`, or `error`).
  static func tell(_ kind: String, _ value: [String: Any]) async -> [String: Any] {
    await Task.detached {
      let json = (try? JSONSerialization.data(withJSONObject: value)).map { String(decoding: $0, as: UTF8.self) } ?? "{}"
      let out = kind.withCString { k in json.withCString { j in oneIOS(k, j) } }
      defer { oneIOSFree(out) }
      guard let out else { return [:] }
      return (try? JSONSerialization.jsonObject(with: Data(String(cString: out).utf8))) as? [String: Any] ?? [:]
    }.value
  }
}

struct SignInArgs: Decodable {
  let url: String
  let scheme: String
}

struct AskArgs: Decodable {
  let ask: Bool
}

struct CountArgs: Decodable {
  let count: Int
}

struct StyleArgs: Decodable {
  let style: String
}

struct GiveArgs: Decodable {
  let path: String
  let preview: Bool
}

struct ValueArgs: Decodable {
  let value: String
}

/// What only Swift does on an iPhone, for the app's Rust, which asks it by name: signing in in the system's sheet,
/// notifications and pushes, the icon's count, haptics, the document camera, files shared or shown, the widgets drawn
/// again, light or dark; and on its own, touch and hold (`Lift`) and what a tapped notification does.
class OnePlugin: Plugin, UNUserNotificationCenterDelegate, ASWebAuthenticationPresentationContextProviding {
  private let lift = Lift()
  private var session: ASWebAuthenticationSession?

  override init() {
    super.init()
    // Before the app has finished starting, so a notification that started it is told here.
    UNUserNotificationCenter.current().delegate = self
    // The person's Appearance from a window's first frame, not a flash of the other one first.
    NotificationCenter.default.addObserver(forName: UIWindow.didBecomeVisibleNotification, object: nil, queue: .main) { note in
      (note.object as? UIWindow).map(Self.appear)
    }
  }

  override func load(webview: WKWebView) {
    lift.attach(webview)
    // Under the page while it draws: the system's background, light or dark, never a white flash.
    webview.isOpaque = false
    webview.backgroundColor = .systemBackground
    webview.scrollView.backgroundColor = .systemBackground
    webview.window.map(Self.appear)
  }

  private var window: UIWindow? { manager.viewController?.view.window }

  /// The view controller on top, to show something over.
  private var top: UIViewController? {
    var top = manager.viewController
    while let next = top?.presentedViewController { top = next }
    return top
  }

  // MARK: Signing in

  /// The sign-in page at `url`, in the system's sign-in sheet, sharing Safari's passkeys and sign-ins → where it came
  /// back to (`<scheme>://signed-in/…`). Closed, it says nothing.
  @objc public func signIn(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(SignInArgs.self)
    guard let url = URL(string: args.url) else { return invoke.reject("That isn’t an address.") }
    DispatchQueue.main.async {
      let session = ASWebAuthenticationSession(url: url, callback: .customScheme(args.scheme)) { [weak self] back, error in
        self?.session = nil
        if let back { return invoke.resolve(["url": back.absoluteString]) }
        if (error as? ASWebAuthenticationSessionError)?.code == .canceledLogin { return invoke.reject("") }
        invoke.reject(error?.localizedDescription ?? "Signing in didn’t work.")
      }
      session.presentationContextProvider = self
      session.prefersEphemeralWebBrowserSession = false
      self.session = session
      session.start()
    }
  }

  func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
    window ?? ASPresentationAnchor()
  }

  // MARK: Notifications

  /// Whether it shows notifications, asking the person first if `ask` and they haven't said → granted, denied or
  /// default.
  @objc public func notify(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(AskArgs.self)
    Task {
      let center = UNUserNotificationCenter.current()
      var status = await center.notificationSettings().authorizationStatus
      if args.ask, status == .notDetermined {
        _ = try? await center.requestAuthorization(options: [.alert, .badge, .sound])
        status = await center.notificationSettings().authorizationStatus
      }
      let state = switch status {
      case .authorized, .provisional, .ephemeral: "granted"
      case .denied: "denied"
      default: "default"
      }
      invoke.resolve(["state": state])
    }
  }

  /// This iPhone's APNs token (hex), for the push Worker; asking the person to notify the first time. A token comes
  /// whatever they answer, for pushes that only wake the app.
  @objc public func push(_ invoke: Invoke) throws {
    DispatchQueue.main.async {
      Tokens.waiting.append { token, failure in
        if let token { invoke.resolve(["token": token]) } else { invoke.reject(failure ?? "No token.") }
      }
      Tokens.listen()
      let center = UNUserNotificationCenter.current()
      center.setNotificationCategories(Notes.categories)
      UIApplication.shared.registerForRemoteNotifications()
      Task {
        if await center.notificationSettings().authorizationStatus == .notDetermined {
          _ = try? await center.requestAuthorization(options: [.alert, .badge, .sound])
        }
      }
    }
  }

  /// What waits for the person, on the app's icon.
  @objc public func badge(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(CountArgs.self)
    UNUserNotificationCenter.current().setBadgeCount(args.count)
    invoke.resolve()
  }

  /// While it's open, a notification shows as it would if it weren't.
  func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
    [.banner, .list, .sound]
  }

  /// A tapped notification opens its screen (the app's Rust takes the link); Approve and Not Me answer the device that
  /// asked to join, as this device, where the person's node checks the request is theirs.
  func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
    let info = response.notification.request.content.userInfo
    if let code = info["join"] as? String, [Notes.approve, Notes.notMe].contains(response.actionIdentifier) {
      let approve = response.actionIdentifier == Notes.approve
      let out = await Rust.tell("join", ["code": code, "approve": approve])
      guard let error = out["error"] as? String else { return }
      // Nothing else is on screen to say so.
      let content = UNMutableNotificationContent()
      content.title = approve ? "Couldn’t approve that device" : "Couldn’t turn that device down"
      content.body = error
      try? await center.add(UNNotificationRequest(identifier: "join \(code)", content: content, trigger: nil))
      return
    }
    guard let link = info["link"] as? String else { return }
    _ = await Rust.tell("open", ["link": link])
  }

  // MARK: Screens

  /// A haptic: "medium" (a long press), "success", "error", or a press. None while a hold is lifting something: the
  /// system gives its own.
  @objc public func haptic(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(StyleArgs.self)
    DispatchQueue.main.async {
      if !self.lift.holding {
        switch args.style {
        case "medium": UIImpactFeedbackGenerator(style: .medium).impactOccurred()
        case "success": UINotificationFeedbackGenerator().notificationOccurred(.success)
        case "error": UINotificationFeedbackGenerator().notificationOccurred(.error)
        default: UISelectionFeedbackGenerator().selectionChanged()
        }
      }
      invoke.resolve()
    }
  }

  /// The document camera → `{ files }`, each page a JPEG; none if the person cancels.
  @objc public func scan(_ invoke: Invoke) throws {
    DispatchQueue.main.async {
      Files.scan(from: self.top) { result in
        switch result {
        case .success(let files): invoke.resolve(["files": files])
        case .failure(let error): invoke.reject(error.localizedDescription)
        }
      }
    }
  }

  /// A file the app wrote (`path`): the share sheet, which saves to Files too; or with `preview`, Quick Look.
  @objc public func give(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(GiveArgs.self)
    DispatchQueue.main.async {
      let url = URL(filePath: args.path)
      if args.preview { Files.preview(url, from: self.top) } else { Files.share(url, from: self.top) }
      invoke.resolve()
    }
  }

  /// A screen's menu: shown with what a touch and hold lifted, if it's for that → `{ index }`; else `{ lifted: false }`.
  @objc public func menu(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(MenuArgs.self)
    DispatchQueue.main.async {
      guard self.lift.waiting else { return invoke.resolve(["lifted": false]) }
      self.lift.show(args.items) { invoke.resolve(["index": $0 as Any]) }
    }
  }

  /// The home-screen widgets drawn again, from what the app just kept for them.
  @objc public func reload(_ invoke: Invoke) throws {
    WidgetCenter.shared.reloadAllTimelines()
    invoke.resolve()
  }

  /// Light, dark or automatic, as the person chose: now, and from the first frame of every start.
  @objc public func appearance(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(ValueArgs.self)
    UserDefaults.standard.set(args.value, forKey: "appearance")
    DispatchQueue.main.async {
      self.window.map(Self.appear)
      invoke.resolve()
    }
  }

  private static func appear(_ window: UIWindow) {
    window.overrideUserInterfaceStyle = switch UserDefaults.standard.string(forKey: "appearance") {
    case "light": .light
    case "dark": .dark
    default: .unspecified
    }
  }
}

/// APNs' answers to registering, which Tauri's app delegate has no place for: added to its class.
enum Tokens {
  static var waiting: [(String?, String?) -> Void] = []
  private static var added = false

  static func listen() {
    guard !added, let delegate = UIApplication.shared.delegate else { return }
    added = true
    let cls: AnyClass = type(of: delegate)
    let ok: @convention(block) (AnyObject, UIApplication, Data) -> Void = { _, _, token in
      answer(token.map { String(format: "%02x", $0) }.joined(), nil)
    }
    let fail: @convention(block) (AnyObject, UIApplication, NSError) -> Void = { _, _, error in
      answer(nil, error.localizedDescription)
    }
    for (selector, block) in [
      (#selector(UIApplicationDelegate.application(_:didRegisterForRemoteNotificationsWithDeviceToken:)), ok as Any),
      (#selector(UIApplicationDelegate.application(_:didFailToRegisterForRemoteNotificationsWithError:)), fail as Any),
    ] {
      let imp = imp_implementationWithBlock(block)
      if !class_addMethod(cls, selector, imp, "v@:@@"), let method = class_getInstanceMethod(cls, selector) {
        method_setImplementation(method, imp)
      }
    }
  }

  private static func answer(_ token: String?, _ failure: String?) {
    let all = waiting
    waiting = []
    all.forEach { $0(token, failure) }
  }
}

/// What the notification extension leaves on each notification (its `Notes`): a device asking to join shows Approve and
/// Not Me; a tap opens `link`, which the app's Rust made.
enum Notes {
  static let join = "join"
  static let approve = "approve"
  static let notMe = "not-me"

  static var categories: Set<UNNotificationCategory> {
    let actions = [
      UNNotificationAction(identifier: approve, title: "Approve", options: [.authenticationRequired]),
      UNNotificationAction(identifier: notMe, title: "Not Me", options: [.authenticationRequired, .destructive]),
    ]
    return [UNNotificationCategory(identifier: join, actions: actions, intentIdentifiers: [])]
  }
}

@_cdecl("init_plugin_one")
func initPlugin() -> Plugin {
  OnePlugin()
}
