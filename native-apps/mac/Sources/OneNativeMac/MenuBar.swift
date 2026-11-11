import AppKit
import SwiftUI

// The Mac's menu bar items, as real menus, so all of a menu's look and behaviour is the system's: its material, corners
// and shadow, its place under the item on any display, the notch, its rows' highlight, separators and checkmarks, closing
// on a click outside or Esc, and never taking the focus. src/platform/menubar.rs says what to show and does what's chosen.

/// What's chosen, to the app.
@_silgen_name("one_menu_chose") private func oneMenuChose(_ json: UnsafePointer<CChar>)

func chose(_ what: [String: Any]) {
    guard let json = try? JSONSerialization.data(withJSONObject: what) else { return }
    String(decoding: json, as: UTF8.self).withCString { oneMenuChose($0) }
}

/// The app's mark, once, with no state yet.
@_cdecl("one_menu_start")
public func oneMenuStart(_ mark: UnsafePointer<UInt8>, _ length: Int) {
    let mark = Data(bytes: mark, count: length)
    DispatchQueue.main.async { MainActor.assumeIsolated { MenuBar.shared.start(mark: mark) } }
}

/// What shows now (`MenuState`, as JSON).
@_cdecl("one_menu_update")
public func oneMenuUpdate(_ json: UnsafePointer<CChar>) {
    let json = Data(String(cString: json).utf8)
    DispatchQueue.main.async {
        MainActor.assumeIsolated {
            guard let state = try? JSONDecoder().decode(MenuState.self, from: json) else { return }
            MenuBar.shared.update(state)
        }
    }
}

struct MenuState: Decodable, Equatable {
    struct Item: Decodable, Equatable {
        var kind: String
        var title: String
        /// Its app's name.
        var label: String
        /// Its headline's first line: none, and it has no item.
        var line: String
        var hidden: Bool
    }
    var name: String
    var signedIn: Bool
    /// This Mac's node.
    var here: String?
    var login: Bool
    var computerUse: Bool
    var items: [Item]
    /// Where the widgets are kept (src/widgets.rs).
    var dir: String
}

/// A menu's row that does something.
private final class Row: NSMenuItem {
    private let run: () -> Void

    init(_ title: String, on: Bool = false, run: @escaping () -> Void) {
        self.run = run
        super.init(title: title, action: #selector(go), keyEquivalent: "")
        target = self
        state = on ? .on : .off
    }

    @available(*, unavailable)
    required init(coder: NSCoder) { fatalError() }

    @objc private func go() { run() }
}

/// A menu's row drawing a view of its own, as wide as the menu.
@MainActor
private func hosted(_ view: some View) -> NSMenuItem {
    let row = NSMenuItem()
    let host = NSHostingView(rootView: view)
    host.frame.size = host.fittingSize
    row.view = host
    return row
}

/// The system's light or dark, whatever the app's window is set to (Tauri's theme is the whole app's).
private func systemAppearance() -> NSAppearance? {
    NSAppearance(named: UserDefaults.standard.string(forKey: "AppleInterfaceStyle") == "Dark" ? .darkAqua : .aqua)
}

@MainActor
final class MenuBar: NSObject, NSMenuDelegate {
    static let shared = MenuBar()

    private let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    private let menu = NSMenu()
    private var state: MenuState?
    private var cache = WidgetCache(widgets: [])
    private var widgetItems: [String: WidgetStatus] = [:]
    private var open = false

    func start(mark: Data) {
        if let image = NSImage(data: mark) {
            image.size = NSSize(width: 18 * image.size.width / image.size.height, height: 18)
            image.isTemplate = true
            item.button?.image = image
        }
        menu.delegate = self
        menu.autoenablesItems = false
        item.menu = menu
        menu.appearance = systemAppearance()
        // A menu takes its appearance before it asks for its items, so it's set when the system's changes, not on opening.
        DistributedNotificationCenter.default().addObserver(self, selector: #selector(systemAppearanceChanged), name: Notification.Name("AppleInterfaceThemeChangedNotification"), object: nil)
    }

    @objc private func systemAppearanceChanged() {
        let appearance = systemAppearance()
        menu.appearance = appearance
        widgetItems.values.forEach { $0.setAppearance(appearance) }
    }

    func update(_ new: MenuState) {
        WidgetFiles.dir = URL(filePath: new.dir, directoryHint: .isDirectory)
        Icons.reload()
        let kept = WidgetCache.load() ?? WidgetCache(widgets: [])
        let changed = new != state || kept.network != cache.network || kept.devices != cache.devices || kept.widgets != cache.widgets
        state = new
        cache = kept
        item.button?.toolTip = new.name
        let shown = new.items.filter { !$0.hidden }
        for (kind, status) in widgetItems where !shown.contains(where: { $0.kind == kind }) {
            status.remove()
            widgetItems[kind] = nil
        }
        for one in shown {
            let status = widgetItems[one.kind] ?? WidgetStatus(kind: one.kind)
            widgetItems[one.kind] = status
            status.show(one, widget: kept.widget(one.kind))
        }
        if open && changed { fill() }
    }

    func menuNeedsUpdate(_ menu: NSMenu) { fill() }

    func menuWillOpen(_ menu: NSMenu) {
        open = true
        chose(["do": "opened"])
    }

    func menuDidClose(_ menu: NSMenu) { open = false }

    private func fill() {
        menu.removeAllItems()
        guard let state else { return menu.addItem(Row("Quit") { chose(["do": "quit"]) }) }
        if state.signedIn {
            header(state)
            devices(state)
            widgets()
        } else {
            let out = NSMenuItem(title: "Signed out", action: nil, keyEquivalent: "")
            out.isEnabled = false
            menu.addItem(out)
        }
        menu.addItem(.separator())
        menu.addItem(Row(state.signedIn ? "Open \(state.name)" : "Sign In…") { chose(["do": "open"]) })
        menu.addItem(Row("Open at Login", on: state.login) { chose(["do": "login"]) })
        let agents = Row("Let Agents Use This Mac", on: state.computerUse) { chose(["do": "computer-use"]) }
        agents.toolTip = "Your own agents may use this Mac's apps, keyboard and pointer. The menu bar shows when one does, with Stop."
        menu.addItem(agents)
        let hidden = state.items.filter(\.hidden)
        if !hidden.isEmpty {
            menu.addItem(.separator())
            for one in hidden { menu.addItem(Row("Show \(one.title) in the Menu Bar") { chose(["do": "show", "kind": one.kind]) }) }
        }
        menu.addItem(.separator())
        menu.addItem(Row("Quit") { chose(["do": "quit"]) })
    }

    private func header(_ state: MenuState) {
        let title = cache.network ?? state.name
        let row = Row(title) { chose(["do": "open"]) }
        row.attributedTitle = NSAttributedString(string: title, attributes: [.font: NSFont.systemFont(ofSize: NSFont.systemFontSize, weight: .semibold)])
        menu.addItem(row)
    }

    private func devices(_ state: MenuState) {
        let here = { (d: WidgetCache.Device) in d.key == state.here }
        let devices = (cache.devices ?? []).sorted { here($0) != here($1) ? here($0) : $0.name.localizedStandardCompare($1.name) == .orderedAscending }
        guard !devices.isEmpty else { return }
        menu.addItem(.sectionHeader(title: "Your Devices"))
        for device in devices {
            let row = Row(device.name) { chose(["do": "devices"]) }
            if here(device) {
                row.badge = NSMenuItemBadge(string: "This device")
            } else if !device.online {
                row.badge = NSMenuItemBadge(string: "Offline")
            }
            menu.addItem(row)
        }
    }

    private func widgets() {
        let widgets = cache.oncePerKind
        guard !widgets.isEmpty else { return }
        menu.addItem(.sectionHeader(title: "Widgets"))
        for widget in widgets { menu.addItem(hosted(WidgetRow(widget: widget, menu: menu))) }
    }
}

/// A widget's own item in the menu bar: its headline, and a menu of the widget.
@MainActor
private final class WidgetStatus: NSObject, NSMenuDelegate {
    private let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    private let menu = NSMenu()
    private var shown: MenuState.Item?
    private var widget: WidgetCache.Item?

    init(kind: String) {
        super.init()
        // Where the user dragged it in the menu bar, kept by macOS under its name.
        item.autosaveName = "widget.\(kind)"
        item.button?.lineBreakMode = .byTruncatingTail
        menu.delegate = self
        menu.autoenablesItems = false
        item.menu = menu
        menu.appearance = systemAppearance()
    }

    func setAppearance(_ appearance: NSAppearance?) { menu.appearance = appearance }

    func show(_ new: MenuState.Item, widget: WidgetCache.Item?) {
        shown = new
        self.widget = widget
        if let button = item.button, button.title != new.line {
            // A new headline pushes the old one up and out.
            if !button.title.isEmpty {
                let push = CATransition()
                push.type = .push
                push.subtype = .fromTop
                push.duration = 0.35
                push.timingFunction = CAMediaTimingFunction(name: .easeOut)
                button.wantsLayer = true
                button.layer?.add(push, forKey: "title")
            }
            button.title = new.line
            button.toolTip = new.title
        }
        item.isVisible = !new.line.isEmpty
    }

    func remove() { NSStatusBar.system.removeStatusItem(item) }

    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        guard let shown else { return }
        if let widget {
            menu.addItem(hosted(WidgetRow(widget: widget, menu: menu)))
            menu.addItem(.separator())
            if widget.opens == true { menu.addItem(Row("Open \(shown.label)") { open(link: widget.link(for: nil)) }) }
        }
        let kind = shown.kind
        menu.addItem(Row("Hide from Menu Bar") { chose(["do": "hide", "kind": kind]) })
    }

    func menuWillOpen(_ menu: NSMenu) { chose(["do": "opened"]) }
}

/// A link followed from a widget, in the app or the browser.
private func open(link: URL?) {
    guard let link else { return }
    chose(["do": "open", "link": link.absoluteString])
}

/// A widget, drawn as on the iPhone at its menu's width. A click on it follows its link or opens its app, closing the
/// menu, as a chosen row does.
private struct WidgetRow: View {
    var widget: WidgetCache.Item
    var menu: NSMenu

    var body: some View {
        let tree = widget.tree("medium")
        VStack(alignment: .leading, spacing: 8) {
            if tree.header != false {
                WidgetLabel(title: widget.title, icon: widget.icon, image: widget.image, imageKind: widget.imageKind)
            }
            if widget.error != nil {
                Label("This widget stopped working.", systemImage: "exclamationmark.triangle.fill").foregroundStyle(Kit.warn)
            } else {
                TimelineView(.everyMinute) { context in
                    WidgetTree(node: tree, now: context.date).font(.system(size: NSFont.systemFontSize))
                }
                .environment(\.widgetHost, WidgetHost(app: widget.app ?? "", scope: widget.scope))
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 6)
        .frame(width: 340, alignment: .leading)
        .contentShape(.rect)
        .onTapGesture { leave(widget.link(for: tree.link)) }
        .environment(\.openURL, OpenURLAction { url in
            leave(url)
            return .handled
        })
    }

    private func leave(_ link: URL?) {
        menu.cancelTracking()
        DispatchQueue.main.async { open(link: link) }
    }
}

extension WidgetCache {
    /// Each widget once, as arranged on the pages.
    var oncePerKind: [Item] {
        var seen = Set<String>()
        return widgets.filter { seen.insert($0.kind ?? $0.id).inserted }
    }

    func widget(_ kind: String) -> Item? { widgets.first { $0.kind == kind } }
}

extension WidgetCache.Item {
    /// Where a click goes: `link`, or its app's first screen if it has one.
    func link(for link: WidgetLink?) -> URL? {
        let target: WidgetLink? = link ?? (opens == true ? WidgetLink.screen("main", params: nil) : nil)
        return target?.url(app: app ?? "", scope: scope)
    }
}
