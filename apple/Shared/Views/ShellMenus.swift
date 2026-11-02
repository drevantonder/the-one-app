import SwiftUI
import WebKit

/// One of a menu's items, as a screen's `one.menu` gives it.
struct ShellMenuItem {
    var title: String
    var danger = false
    var checked = false

    static func from(_ list: Any?) -> [ShellMenuItem] {
        (list as? [[String: Any]] ?? []).map {
            ShellMenuItem(title: $0["title"] as? String ?? "", danger: $0["danger"] as? Bool ?? false, checked: $0["checked"] as? Bool ?? false)
        }
    }
}

#if os(iOS)
/// A screen's menus, the iPhone's own: touch and hold lifts what's held out of the page with its menu, as the Home
/// Screen does; a menu asked for otherwise (a button's) opens where it was asked for.
///
/// What's held is known before the hold is: as a finger comes down, the page says what's under it that listens for
/// `contextmenu` (`script`). Once the hold lifts it, the page is sent a `contextmenu` there, which it answers with
/// `one.menu`, and that's the menu shown.
@MainActor
final class ShellMenus: NSObject, UIContextMenuInteractionDelegate {
    weak var web: WKWebView?
    /// What a finger is on: its frame in the web view, and its corners' radius.
    private var under: (rect: CGRect, radius: CGFloat)?
    private var hold: Hold?

    /// Before the page's scripts: marks what listens for `contextmenu`, and says what's under each finger that comes down.
    static let script = """
    (() => {
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

    /// `one.menu`: the held thing's, once its hold asked the page; otherwise at `point`. → the index chosen, or nil.
    func show(_ items: [ShellMenuItem], at point: CGPoint) async -> Int? {
        if let hold, hold.items == nil {
            if hold.over { return nil }
            hold.items = items
            hold.ready?(items)
            return await withCheckedContinuation { done in hold.answer = { done.resume(returning: $0) } }
        }
        return await withCheckedContinuation { done in present(items, at: point) { done.resume(returning: $0) } }
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

    // MARK: Menus at a point

    private func present(_ items: [ShellMenuItem], at point: CGPoint, done: @escaping (Int?) -> Void) {
        guard let web, !items.isEmpty else { return done(nil) }
        let button = MenuButton(frame: CGRect(x: point.x - 1, y: point.y - 1, width: 2, height: 2))
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

    /// The items, as the iPhone lays a menu out: what can't be undone in a section of its own, at the end.
    private func elements(_ items: [ShellMenuItem], chose: @escaping (Int) -> Void) -> [UIMenuElement] {
        let actions = items.enumerated().map { i, item in
            UIAction(title: item.title, attributes: item.danger ? .destructive : [], state: item.checked ? .on : .off) { _ in chose(i) }
        }
        let safe = actions.enumerated().filter { !items[$0.offset].danger }.map(\.element)
        let danger = actions.enumerated().filter { items[$0.offset].danger }.map(\.element)
        if safe.isEmpty || danger.isEmpty { return actions }
        return [UIMenu(options: .displayInline, children: safe), UIMenu(options: .displayInline, children: danger)]
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
        if i > 0, item.danger, !items[i - 1].danger { menu.addItem(.separator()) }
        let entry = NSMenuItem(title: item.title, action: #selector(Chooser.choose(_:)), keyEquivalent: "")
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
