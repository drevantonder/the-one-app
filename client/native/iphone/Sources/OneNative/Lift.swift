import UIKit
import WebKit

struct MenuArgs: Decodable {
  struct Item: Decodable {
    let title: String
    let danger: Bool?
    let checked: Bool?
    let icon: String?
    let row: Bool?
    let section: Bool?
  }
  let items: [Item]
}

/// Touch and hold, the iPhone's own: what's held lifts out of the page with its menu, as on the Home Screen, and a finger
/// that moves it carries it.
///
/// What a screen lets be held it marks `data-one-held` (the home's pages do), which it can be told knows this
/// (`__oneLifts`, the page sets it in its frames). As a finger comes down in a screen's frame, a script of the app's own,
/// in a world of its own that the screen's can't reach, says what's under it (`oneHeld`). Once the hold lifts it, the
/// screen is sent a `contextmenu` there, which it answers with `one.menu`, and the page passes that here (`show`): the
/// menu shown with it.
final class Lift: NSObject, UIContextMenuInteractionDelegate, UIGestureRecognizerDelegate, WKScriptMessageHandler {
  private weak var web: WKWebView?
  /// What a finger is on: its frame in its screen's frame, its corners' radius, where the finger was there, and which
  /// frame.
  private var under: (rect: CGRect, radius: CGFloat, at: CGPoint, frame: WKFrameInfo)?
  private var hold: Hold?

  private static let world = WKContentWorld.world(name: "one-lift")
  private static let script = """
  addEventListener("touchstart", (e) => {
    const t = e.touches[0], el = e.target.closest?.("[data-one-held]"), r = el?.getBoundingClientRect();
    webkit.messageHandlers.oneHeld.postMessage(r && t ? { rect: [r.x, r.y, r.width, r.height, parseFloat(getComputedStyle(el).borderTopLeftRadius) || 0], at: [t.clientX, t.clientY] } : {});
  }, { capture: true, passive: true });
  """

  /// A hold, from its lift until its menu is gone.
  private final class Hold {
    let rect: CGRect
    let radius: CGFloat
    let snapshot: UIView?
    /// Where the screen's frame is in the web view, and the frame.
    let offset: CGPoint
    let frame: WKFrameInfo
    var items: [MenuArgs.Item]?
    var ready: (([MenuArgs.Item]) -> Void)?
    var answer: ((Int?) -> Void)?
    var answered = false
    var over = false

    init(rect: CGRect, radius: CGFloat, snapshot: UIView?, offset: CGPoint, frame: WKFrameInfo) {
      self.rect = rect
      self.radius = radius
      self.snapshot = snapshot
      self.offset = offset
      self.frame = frame
    }

    func finish(_ chose: Int?) {
      guard !answered else { return }
      answered = true
      answer?(chose)
    }
  }

  func attach(_ web: WKWebView) {
    self.web = web
    let content = web.configuration.userContentController
    content.add(self, contentWorld: Self.world, name: "oneHeld")
    content.addUserScript(WKUserScript(source: Self.script, injectionTime: .atDocumentStart, forMainFrameOnly: false, in: Self.world))
    web.addInteraction(UIContextMenuInteraction(delegate: self))
    web.addGestureRecognizer(carry)
  }

  /// Whether a hold is lifting or showing its menu: the page's own haptic for it is the system's.
  var holding: Bool { hold.map { !$0.over } ?? false }

  /// Whether a hold waits for its menu from the page.
  var waiting: Bool { hold.map { !$0.over && $0.items == nil } ?? false }

  func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
    guard let said = message.body as? [String: Any], let r = said["rect"] as? [Double], r.count == 5, let at = said["at"] as? [Double], at.count == 2 else {
      under = nil
      return
    }
    under = (CGRect(x: r[0], y: r[1], width: r[2], height: r[3]), max(r[4], 12), CGPoint(x: at[0], y: at[1]), message.frameInfo)
  }

  /// The held thing's menu, which its hold asked the page for → the index chosen, or nil.
  func show(_ items: [MenuArgs.Item], done: @escaping (Int?) -> Void) {
    guard let hold, !hold.over, hold.items == nil else { return done(nil) }
    hold.items = items
    hold.answer = done
    hold.ready?(items)
  }

  /// Runs `js` in the held thing's screen, as the screen's own.
  private func run(_ js: String, in frame: WKFrameInfo) {
    web?.evaluateJavaScript(js, in: frame, in: .page) { _ in }
  }

  // MARK: Carrying what's held

  /// A finger that moves what a hold lifted carries it, as on the Home Screen: the menu goes, and the screen (which
  /// starts arranging) is told where the finger is until it lets go (`__oneDrag`).
  private lazy var carry: UILongPressGestureRecognizer = {
    let g = UILongPressGestureRecognizer(target: self, action: #selector(carried(_:)))
    g.minimumPressDuration = 0.35
    g.allowableMovement = .greatestFiniteMagnitude
    g.cancelsTouchesInView = false
    g.delaysTouchesEnded = false
    g.delegate = self
    return g
  }()
  private var carrying: Hold?
  private var start: CGPoint?

  @objc private func carried(_ g: UILongPressGestureRecognizer) {
    guard let web else { return }
    let at = g.location(in: web)
    let drag = { (hold: Hold, phase: String) in
      self.run("window.__oneDrag && __oneDrag(\"\(phase)\", \(at.x - hold.offset.x), \(at.y - hold.offset.y))", in: hold.frame)
    }
    switch g.state {
    case .began:
      start = at
    case .changed:
      if let carrying { return drag(carrying, "move") }
      // Only what a hold lifted, once it's moved far enough to be more than a wobble.
      guard let start, let hold, !hold.over, hypot(at.x - start.x, at.y - start.y) > 12 else { return }
      carrying = hold
      hold.finish(nil)
      hold.items = hold.items ?? []
      web.interactions.compactMap { $0 as? UIContextMenuInteraction }.forEach { $0.dismissMenu() }
      drag(hold, "start")
    default:
      if let carrying { drag(carrying, "end") }
      carrying = nil
      start = nil
    }
  }

  func gestureRecognizer(_ g: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool { true }

  // MARK: Touch and hold

  func contextMenuInteraction(_ interaction: UIContextMenuInteraction, configurationForMenuAtLocation location: CGPoint) -> UIContextMenuConfiguration? {
    guard let web, let under else { return nil }
    // Where the screen's frame is: where the finger is here, less where it was in the frame.
    let offset = CGPoint(x: location.x - under.at.x, y: location.y - under.at.y)
    let rect = under.rect.offsetBy(dx: offset.x, dy: offset.y)
    guard rect.insetBy(dx: -8, dy: -8).contains(location) else { return nil }
    let hold = Hold(rect: rect, radius: under.radius, snapshot: web.resizableSnapshotView(from: rect, afterScreenUpdates: false, withCapInsets: .zero),
                    offset: offset, frame: under.frame)
    self.hold = hold
    run("""
    (() => {
      const el = document.elementFromPoint(\(under.at.x), \(under.at.y));
      document.querySelectorAll("[data-one-lifted]").forEach((e) => e.removeAttribute("data-one-lifted"));
      el?.closest("[data-one-held]")?.setAttribute("data-one-lifted", "");
      el?.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: \(under.at.x), clientY: \(under.at.y) }));
    })();
    """, in: under.frame)
    return UIContextMenuConfiguration(identifier: "held" as NSString, previewProvider: nil) { [weak self] _ in
      if let items = hold.items { return UIMenu(children: self?.elements(items) { hold.finish($0) } ?? []) }
      // The page hasn't answered yet: its menu fills in when it does, or it goes.
      return UIMenu(children: [UIDeferredMenuElement.uncached { fill in
        hold.ready = { [weak self] items in fill(self?.elements(items) { hold.finish($0) } ?? []) }
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
          guard hold.items == nil else { return }
          hold.ready = nil
          fill([])
          interaction.dismissMenu()
        }
      }])
    }
  }

  func contextMenuInteraction(_ interaction: UIContextMenuInteraction, configuration: UIContextMenuConfiguration,
                              highlightPreviewForItemWithIdentifier identifier: any NSCopying) -> UITargetedPreview? {
    hold.flatMap(preview)
  }

  func contextMenuInteraction(_ interaction: UIContextMenuInteraction, configuration: UIContextMenuConfiguration,
                              dismissalPreviewForItemWithIdentifier identifier: any NSCopying) -> UITargetedPreview? {
    hold.flatMap(preview)
  }

  /// While it's lifted, what's held is gone from the screen, as an icon is from the Home Screen.
  func contextMenuInteraction(_ interaction: UIContextMenuInteraction, willDisplayMenuFor configuration: UIContextMenuConfiguration,
                              animator: (any UIContextMenuInteractionAnimating)?) {
    guard let hold else { return }
    run(#"document.querySelector("[data-one-lifted]")?.style.setProperty("visibility", "hidden")"#, in: hold.frame)
  }

  func contextMenuInteraction(_ interaction: UIContextMenuInteraction, willEndFor configuration: UIContextMenuConfiguration,
                              animator: (any UIContextMenuInteractionAnimating)?) {
    guard let hold else { return }
    hold.over = true
    // A choice's action comes once the menu has gone; nothing chosen by then, it's nothing.
    let ended = { [weak self] in
      self?.run(#"""
      document.querySelectorAll("[data-one-lifted]").forEach((e) => { e.style.removeProperty("visibility"); e.removeAttribute("data-one-lifted"); });
      """#, in: hold.frame)
      DispatchQueue.main.asyncAfter(deadline: .now() + 0.6) {
        hold.finish(nil)
        if self?.hold === hold { self?.hold = nil }
      }
    }
    if let animator { animator.addCompletion(ended) } else { ended() }
  }

  /// What's held, lifted out of the screen: a picture of it, in place, with its corners.
  private func preview(_ hold: Hold) -> UITargetedPreview? {
    guard let web, let snapshot = hold.snapshot else { return nil }
    let look = UIPreviewParameters()
    look.visiblePath = UIBezierPath(roundedRect: CGRect(origin: .zero, size: hold.rect.size), cornerRadius: hold.radius)
    look.backgroundColor = .clear
    return UITargetedPreview(view: snapshot, parameters: look, target: UIPreviewTarget(container: web, center: CGPoint(x: hold.rect.midX, y: hold.rect.midY)))
  }

  /// The system's symbol for the Phosphor icons menus name.
  private static let symbols = [
    "dots-nine": "square.grid.3x3", "info": "info.circle", "minus-circle": "minus.circle", "copy": "doc.on.doc", "gear-six": "gearshape",
    "bag": "bag", "trash": "trash", "list-bullets": "list.bullet", "square": "square", "rectangle": "rectangle", "squares-four": "square.grid.2x2",
    "columns": "rectangle.split.2x1", "plus-circle": "plus.circle", "plus-square": "plus.rectangle.on.rectangle", "pencil-simple": "pencil",
    "warning": "exclamationmark.triangle", "arrow-square-out": "arrow.up.right.square",
  ]

  /// The items, as the iPhone lays a menu out: the row's choices across the top, as the Home Screen's sizes are, then
  /// each section, what can't be undone in one of its own, at the end.
  private func elements(_ items: [MenuArgs.Item], chose: @escaping (Int) -> Void) -> [UIMenuElement] {
    let actions = items.enumerated().map { i, item in
      UIAction(title: item.title, image: item.icon.flatMap { Self.symbols[$0] }.flatMap { UIImage(systemName: $0) },
               attributes: item.danger == true ? .destructive : [], state: item.checked == true ? .on : .off) { _ in chose(i) }
    }
    var out: [UIMenuElement] = []
    let row = items.indices.filter { items[$0].row == true }
    if !row.isEmpty { out.append(UIMenu(options: [.displayInline, .displayAsPalette], children: row.map { actions[$0] })) }
    var sections: [[Int]] = []
    for i in items.indices where items[i].row != true {
      let danger = items[i].danger == true
      let fresh = sections.isEmpty || items[i].section == true || (danger && !(sections.last?.last.map { items[$0].danger == true } ?? true))
      if fresh { sections.append([i]) } else { sections[sections.count - 1].append(i) }
    }
    out += sections.map { UIMenu(options: .displayInline, children: $0.map { actions[$0] }) }
    return out.count == 1 ? (out[0] as? UIMenu)?.children ?? out : out
  }
}
