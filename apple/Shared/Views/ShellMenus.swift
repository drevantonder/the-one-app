import SwiftUI
import WebKit

/// One of a menu's items, as a screen's `one.menu` gives it: its Phosphor `icon`, and whether it's one of the choices
/// in a row at the top (`row`, a widget's sizes) or starts a section of its own.
struct ShellMenuItem {
    var title: String
    var danger = false
    var checked = false
    var icon: String?
    var row = false
    var section = false

    static func from(_ list: Any?) -> [ShellMenuItem] {
        (list as? [[String: Any]] ?? []).map {
            ShellMenuItem(title: $0["title"] as? String ?? "", danger: $0["danger"] as? Bool ?? false, checked: $0["checked"] as? Bool ?? false,
                          icon: ($0["icon"] as? String).flatMap { $0.isEmpty ? nil : $0 }, row: $0["row"] as? Bool ?? false, section: $0["section"] as? Bool ?? false)
        }
    }

    /// The system's symbol for the Phosphor icons menus name, as the native screens use them.
    static let symbols = [
        "dots-nine": "square.grid.3x3", "info": "info.circle", "minus-circle": "minus.circle", "copy": "doc.on.doc", "gear-six": "gearshape",
        "bag": "bag", "trash": "trash", "list-bullets": "list.bullet", "square": "square", "rectangle": "rectangle", "squares-four": "square.grid.2x2",
        "columns": "rectangle.split.2x1", "plus-circle": "plus.circle", "plus-square": "plus.rectangle.on.rectangle",
    ]

    #if os(iOS)
    typealias Image = UIImage
    #else
    typealias Image = NSImage
    #endif

    /// Its icon: the system's symbol for it, or the Phosphor icon itself, drawn as a template.
    var image: Image? {
        guard let icon else { return nil }
        if let name = Self.symbols[icon] { return Image(systemName: name) }
        guard let shape = Icons.shape(icon) else { return nil }
        let size = CGSize(width: 20, height: 20)
        let path = shape.path(in: CGRect(origin: .zero, size: size)).cgPath
        #if os(iOS)
        return UIGraphicsImageRenderer(size: size).image { c in
            c.cgContext.addPath(path)
            c.cgContext.fillPath()
        }.withRenderingMode(.alwaysTemplate)
        #else
        let image = NSImage(size: size, flipped: true) { _ in
            guard let c = NSGraphicsContext.current?.cgContext else { return false }
            c.addPath(path)
            c.fillPath()
            return true
        }
        image.isTemplate = true
        return image
        #endif
    }
}

#if os(macOS)
private extension NSImage {
    convenience init?(systemName: String) { self.init(systemSymbolName: systemName, accessibilityDescription: nil) }
}
#endif

#if os(iOS)
/// A screen's menus, the iPhone's own: touch and hold lifts what's held out of the page with its menu, as the Home
/// Screen does; a menu asked for otherwise (a button's) opens where it was asked for.
///
/// What's held is known before the hold is: as a finger comes down, the page says what's under it that listens for
/// `contextmenu` (`script`). Once the hold lifts it, the page is sent a `contextmenu` there, which it answers with
/// `one.menu`, and that's the menu shown.
@MainActor
final class ShellMenus: NSObject, UIContextMenuInteractionDelegate, UIGestureRecognizerDelegate {
    weak var web: WKWebView?
    /// What a finger is on: its frame in the web view, and its corners' radius.
    private var under: (rect: CGRect, radius: CGFloat)?
    private var hold: Hold?

    /// Before the page's scripts: marks what listens for `contextmenu`, and says what's under each finger that comes down.
    static let script = """
    (() => {
      window.__oneLifts = true;
      const add = EventTarget.prototype.addEventListener;
      EventTarget.prototype.addEventListener = function (type, ...rest) {
        if (type === "contextmenu" && this instanceof Element) this.setAttribute("data-one-held", "");
        return add.call(this, type, ...rest);
      };
      add.call(document, "touchstart", (e) => {
        const el = e.target.closest?.("[data-one-held]"), r = el?.getBoundingClientRect();
        const rect = r ? [r.x, r.y, r.width, r.height, parseFloat(getComputedStyle(el).borderTopLeftRadius) || 0] : null;
        window.webkit.messageHandlers.one.postMessage({ type: "held", rect }).catch(() => {});
      }, { capture: true, passive: true });
    })();
    """

    /// A hold, from its lift until its menu is gone.
    private final class Hold {
        let rect: CGRect
        let radius: CGFloat
        let snapshot: UIView?
        var items: [ShellMenuItem]?
        var ready: (([ShellMenuItem]) -> Void)?
        var answer: ((Int?) -> Void)?
        var answered = false
        var over = false

        init(rect: CGRect, radius: CGFloat, snapshot: UIView?) {
            self.rect = rect
            self.radius = radius
            self.snapshot = snapshot
        }

        func finish(_ chose: Int?) {
            guard !answered else { return }
            answered = true
            answer?(chose)
        }
    }

    /// Whether a hold is lifting or showing its menu: the page's own haptic for it is the system's.
    var holding: Bool { hold.map { !$0.over } ?? false }

    /// The page's word on what's under a finger: `[x, y, width, height, radius]`, or nothing.
    func held(_ rect: Any?) {
        guard let r = rect as? [Double], r.count == 5 else { under = nil; return }
        under = (CGRect(x: r[0], y: r[1], width: r[2], height: r[3]), max(r[4], 12))
    }

    /// `one.menu`: the held thing's, once its hold asked the page; otherwise from `rect` (the element it's for), or
    /// at `point`. → the index chosen, or nil.
    func show(_ items: [ShellMenuItem], at point: CGPoint, from rect: CGRect? = nil) async -> Int? {
        if let hold, hold.items == nil {
            if hold.over { return nil }
            hold.items = items
            hold.ready?(items)
            return await withCheckedContinuation { done in hold.answer = { done.resume(returning: $0) } }
        }
        let frame = rect ?? CGRect(x: point.x - 1, y: point.y - 1, width: 2, height: 2)
        return await withCheckedContinuation { done in present(items, from: frame) { done.resume(returning: $0) } }
    }

    // MARK: Carrying what's held

    /// A finger that moves what a hold lifted carries it, as on the Home Screen: the menu goes, and the page (which
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
    private var carrying = false
    private var start: CGPoint?

    func watch(_ web: WKWebView) {
        if carry.view !== web { web.addGestureRecognizer(carry) }
    }

    @objc private func carried(_ g: UILongPressGestureRecognizer) {
        guard let web else { return }
        let at = g.location(in: web)
        let call = { (phase: String) in web.evaluateJavaScript("window.__oneDrag && __oneDrag(\"\(phase)\", \(at.x), \(at.y))") }
        switch g.state {
        case .began:
            start = at
        case .changed:
            if carrying { call("move"); return }
            // Only what a hold lifted, once it's moved far enough to be more than a wobble.
            guard let start, let hold, !hold.over, hypot(at.x - start.x, at.y - start.y) > 12 else { return }
            carrying = true
            hold.finish(nil)
            hold.items = hold.items ?? []
            web.interactions.compactMap { $0 as? UIContextMenuInteraction }.forEach { $0.dismissMenu() }
            web.evaluateJavaScript("window.__oneDrag && __oneDrag(\"start\", \(at.x), \(at.y))")
        default:
            if carrying { call("end") }
            carrying = false
            start = nil
        }
    }

    // MARK: Touch and hold

    func contextMenuInteraction(_ interaction: UIContextMenuInteraction, configurationForMenuAtLocation location: CGPoint) -> UIContextMenuConfiguration? {
        guard let web, let under, under.rect.contains(location) else { return nil }
        let hold = Hold(rect: under.rect, radius: under.radius, snapshot: web.resizableSnapshotView(from: under.rect, afterScreenUpdates: false, withCapInsets: .zero))
        self.hold = hold
        web.evaluateJavaScript("""
        (() => {
          const el = document.elementFromPoint(\(location.x), \(location.y));
          document.querySelectorAll("[data-one-lifted]").forEach((e) => e.removeAttribute("data-one-lifted"));
          el?.closest("[data-one-held]")?.setAttribute("data-one-lifted", "");
          el?.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: \(location.x), clientY: \(location.y) }));
        })();
        """)
        return UIContextMenuConfiguration(identifier: "held" as NSString, previewProvider: nil) { [weak self] _ in
            if let items = hold.items { return UIMenu(children: self?.elements(items) { hold.finish($0) } ?? []) }
            // The page hasn't answered yet: its menu fills in when it does, or it goes.
            return UIMenu(children: [UIDeferredMenuElement.uncached { fill in
                MainActor.assumeIsolated {
                    hold.ready = { [weak self] items in fill(self?.elements(items) { hold.finish($0) } ?? []) }
                    DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
                        guard hold.items == nil else { return }
                        hold.ready = nil
                        fill([])
                        interaction.dismissMenu()
                    }
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

    /// While it's lifted, what's held is gone from the page, as an icon is from the Home Screen.
    func contextMenuInteraction(_ interaction: UIContextMenuInteraction, willDisplayMenuFor configuration: UIContextMenuConfiguration,
                                animator: (any UIContextMenuInteractionAnimating)?) {
        web?.evaluateJavaScript(#"document.querySelector("[data-one-lifted]")?.style.setProperty("visibility", "hidden")"#)
    }

    func contextMenuInteraction(_ interaction: UIContextMenuInteraction, willEndFor configuration: UIContextMenuConfiguration,
                                animator: (any UIContextMenuInteractionAnimating)?) {
        guard let hold else { return }
        hold.over = true
        // A choice's action comes once the menu has gone; nothing chosen by then, it's nothing.
        let ended = { [weak self] in
            self?.web?.evaluateJavaScript(#"""
            document.querySelectorAll("[data-one-lifted]").forEach((e) => { e.style.removeProperty("visibility"); e.removeAttribute("data-one-lifted"); });
            """#)
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.6) {
                hold.finish(nil)
                if self?.hold === hold { self?.hold = nil }
            }
        }
        if let animator { animator.addCompletion(ended) } else { ended() }
    }

    /// What's held, lifted out of the page: a picture of it, in place, with its corners.
    private func preview(_ hold: Hold) -> UITargetedPreview? {
        guard let web, let snapshot = hold.snapshot else { return nil }
        let look = UIPreviewParameters()
        look.visiblePath = UIBezierPath(roundedRect: CGRect(origin: .zero, size: hold.rect.size), cornerRadius: hold.radius)
        look.backgroundColor = .clear
        return UITargetedPreview(view: snapshot, parameters: look, target: UIPreviewTarget(container: web, center: CGPoint(x: hold.rect.midX, y: hold.rect.midY)))
    }

    func gestureRecognizer(_ g: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool { true }

    // MARK: Menus at a point

    private func present(_ items: [ShellMenuItem], from frame: CGRect, done: @escaping (Int?) -> Void) {
        guard let web, !items.isEmpty else { return done(nil) }
        let button = MenuButton(frame: frame)
        button.done = done
        button.menu = UIMenu(children: elements(items) { [weak button] in button?.finish($0) })
        button.showsMenuAsPrimaryAction = true
        button.preferredMenuElementOrder = .fixed
        web.addSubview(button)
        button.performPrimaryAction()
    }

    /// A menu that opens at a point, as a button's: a button there of no size, pressed.
    private final class MenuButton: UIButton {
        var done: ((Int?) -> Void)?

        func finish(_ chose: Int?) {
            done?(chose)
            done = nil
        }

        override func contextMenuInteraction(_ interaction: UIContextMenuInteraction, willEndFor configuration: UIContextMenuConfiguration,
                                             animator: (any UIContextMenuInteractionAnimating)?) {
            super.contextMenuInteraction(interaction, willEndFor: configuration, animator: animator)
            let ended = { [weak self] in
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.6) {
                    self?.finish(nil)
                    self?.removeFromSuperview()
                }
            }
            if let animator { animator.addCompletion(ended) } else { ended() }
        }
    }

    /// The items, as the iPhone lays a menu out: the row's choices across the top, as the Home Screen's sizes are,
    /// then each section, what can't be undone in one of its own, at the end.
    private func elements(_ items: [ShellMenuItem], chose: @escaping (Int) -> Void) -> [UIMenuElement] {
        let actions = items.enumerated().map { i, item in
            UIAction(title: item.title, image: item.image, attributes: item.danger ? .destructive : [], state: item.checked ? .on : .off) { _ in chose(i) }
        }
        var out: [UIMenuElement] = []
        let row = items.indices.filter { items[$0].row }
        if !row.isEmpty { out.append(UIMenu(options: [.displayInline, .displayAsPalette], children: row.map { actions[$0] })) }
        var sections: [[Int]] = []
        for i in items.indices where !items[i].row {
            let fresh = sections.isEmpty || items[i].section || (items[i].danger && !(sections.last?.last.map { items[$0].danger } ?? true))
            if fresh { sections.append([i]) } else { sections[sections.count - 1].append(i) }
        }
        out += sections.map { UIMenu(options: .displayInline, children: $0.map { actions[$0] }) }
        return out.count == 1 ? (out[0] as? UIMenu)?.children ?? out : out
    }
}
#endif

/// A screen's `one.alert`, the system's own: its title and text with OK, or with `action`, Cancel and that (in red
/// if it can't be undone). → whether it was OK'd or the action chosen.
@MainActor
func shellAlert(title: String, text: String, action: String?, danger: Bool, from web: WKWebView?) async -> Bool {
    #if os(iOS)
    await withCheckedContinuation { done in
        let alert = UIAlertController(title: title.isEmpty ? nil : title, message: text.isEmpty ? nil : text, preferredStyle: .alert)
        if let action {
            alert.addAction(UIAlertAction(title: String(localized: "Cancel"), style: .cancel) { _ in done.resume(returning: false) })
            alert.addAction(UIAlertAction(title: action, style: danger ? .destructive : .default) { _ in done.resume(returning: true) })
        } else {
            alert.addAction(UIAlertAction(title: String(localized: "OK"), style: .default) { _ in done.resume(returning: true) })
        }
        guard let top = ShellFiles.top(web) else { return done.resume(returning: false) }
        top.present(alert, animated: true)
    }
    #else
    let alert = NSAlert()
    alert.messageText = title
    alert.informativeText = text
    if let action {
        alert.addButton(withTitle: action).hasDestructiveAction = danger
        alert.addButton(withTitle: String(localized: "Cancel"))
    } else {
        alert.addButton(withTitle: String(localized: "OK"))
    }
    let answer = if let window = web?.window { await alert.beginSheetModal(for: window) } else { alert.runModal() }
    return answer == .alertFirstButtonReturn
    #endif
}

#if os(macOS)
/// A screen's `one.menu` on a Mac: the menu, at `point` in the page. → the index chosen, or nil.
@MainActor
func shellMenu(_ items: [ShellMenuItem], at point: CGPoint, in web: WKWebView?) -> Int? {
    guard let web, !items.isEmpty else { return nil }
    final class Chooser: NSObject {
        var chose: Int?
        @objc func choose(_ item: NSMenuItem) { chose = item.tag }
    }
    let chooser = Chooser()
    let menu = NSMenu()
    for (i, item) in items.enumerated() {
        if i > 0, (item.danger && !items[i - 1].danger) || item.section || (items[i - 1].row && !item.row) { menu.addItem(.separator()) }
        let entry = NSMenuItem(title: item.title, action: #selector(Chooser.choose(_:)), keyEquivalent: "")
        entry.image = item.image
        entry.target = chooser
        entry.tag = i
        entry.state = item.checked ? .on : .off
        if item.danger { entry.attributedTitle = NSAttributedString(string: item.title, attributes: [.foregroundColor: NSColor.systemRed]) }
        menu.addItem(entry)
    }
    let at = web.isFlipped ? point : CGPoint(x: point.x, y: web.bounds.height - point.y)
    menu.popUp(positioning: nil, at: at, in: web)
    return chooser.chose
}
#endif
