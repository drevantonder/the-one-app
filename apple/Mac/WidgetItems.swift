import AppKit
import SwiftUI

/// A menu bar item of its own for each widget on the pages, beside the app's: titled with the widget's
/// headline, it opens the widget, drawn natively, below it. A new headline pushes the old one up and out.
@MainActor @Observable
final class WidgetItems {
    static let shared = WidgetItems()

    /// The widgets whose items someone hid.
    private(set) var hidden = Set(UserDefaults.standard.stringArray(forKey: "hiddenWidgets") ?? [])
    @ObservationIgnored var openWindow: (() -> Void)?
    @ObservationIgnored private var store: Store?
    @ObservationIgnored private var items: [String: WidgetItem] = [:]

    func start(_ store: Store) {
        self.store = store
        watch()
    }

    func toggle(_ id: String) {
        if hidden.contains(id) { hidden.remove(id) } else { hidden.insert(id) }
        UserDefaults.standard.set(Array(hidden), forKey: "hiddenWidgets")
    }

    /// Opens an app's first screen in the window.
    func open(_ app: String) {
        guard let store else { return }
        if let app = store.app(app) { store.selection = store.open(app, from: store.pages.first { store.apps(on: $0).contains(app) }) }
        openWindow?()
        NSApp.activate()
    }

    private func watch() {
        withObservationTracking { update() } onChange: {
            Task { @MainActor in self.watch() }
        }
    }

    private func update() {
        guard let store else { return }
        let shown = store.allWidgets.filter { !hidden.contains($0.id) }
        for (id, item) in items where !shown.contains(where: { $0.id == id }) {
            item.remove()
            items[id] = nil
        }
        for widget in shown {
            let item = items[widget.id] ?? WidgetItem(id: widget.id, owner: self)
            items[widget.id] = item
            item.show(widget, app: store.app(widget.app))
        }
    }
}

@MainActor
private final class WidgetItem: NSObject {
    private let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    private let popover = NSPopover()
    private let shown = Shown()
    private unowned let owner: WidgetItems

    @MainActor @Observable
    final class Shown {
        var widget: WidgetInfo?
        var app: AppInfo?
    }

    init(id: String, owner: WidgetItems) {
        self.owner = owner
        super.init()
        item.autosaveName = "widget.\(id)"
        popover.behavior = .transient
        popover.animates = false
        let host = NSHostingController(rootView: WidgetPopover(shown: shown, open: { [weak self] in self?.openApp() }))
        host.sizingOptions = .preferredContentSize
        popover.contentViewController = host
        item.button?.target = self
        item.button?.action = #selector(click)
        item.button?.sendAction(on: [.leftMouseUp, .rightMouseUp])
        item.button?.lineBreakMode = .byTruncatingTail
    }

    func show(_ widget: WidgetInfo, app: AppInfo?) {
        shown.widget = widget
        shown.app = app
        let line = widget.root.headline?.split(separator: "\n").first.map(String.init) ?? ""
        let title = line.count > 40 ? line.prefix(39).trimmingCharacters(in: .whitespaces) + "…" : line
        if let button = item.button, button.title != title {
            if !button.title.isEmpty {
                let push = CATransition()
                push.type = .push
                push.subtype = .fromTop
                push.duration = 0.35
                push.timingFunction = CAMediaTimingFunction(name: .easeOut)
                button.wantsLayer = true
                button.layer?.add(push, forKey: "title")
            }
            button.title = title
            button.toolTip = widget.title
        }
        item.isVisible = !title.isEmpty
    }

    func remove() {
        popover.performClose(nil)
        NSStatusBar.system.removeStatusItem(item)
    }

    @objc private func click(_ sender: NSStatusBarButton) {
        if NSApp.currentEvent?.type == .rightMouseUp {
            let menu = NSMenu()
            if let app = shown.app { menu.addItem(withTitle: "Open \(app.label)", action: #selector(openApp), keyEquivalent: "").target = self }
            menu.addItem(withTitle: "Hide from Menu Bar", action: #selector(hide), keyEquivalent: "").target = self
            menu.popUp(positioning: nil, at: NSPoint(x: 0, y: sender.bounds.height + 5), in: sender)
        } else if popover.isShown {
            popover.performClose(sender)
        } else {
            NSApp.activate()
            popover.show(relativeTo: sender.bounds, of: sender, preferredEdge: .minY)
        }
    }

    @objc private func openApp() {
        popover.performClose(nil)
        if let app = shown.app { owner.open(app.name) }
    }

    @objc private func hide() {
        if let id = shown.widget?.id { owner.toggle(id) }
    }
}

private struct WidgetPopover: View {
    var shown: WidgetItem.Shown
    var open: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Label(shown.widget?.title ?? shown.app?.label ?? "", systemImage: symbol(shown.app?.icon))
                    .font(.headline)
                Spacer()
                if let app = shown.app, app.hasScreens {
                    Button("Open", action: open)
                        .buttonStyle(.borderless)
                        .help("Open \(app.label)")
                }
            }
            if let root = shown.widget?.root {
                TimelineView(.everyMinute) { context in
                    WidgetTree(node: root, now: context.date)
                }
            }
        }
        .padding(14)
        .frame(width: 340, alignment: .leading)
        .tint(Color(brandHex: accentHex))
    }

    private var accentHex: String { Bundle.main.object(forInfoDictionaryKey: "BrandAccent") as? String ?? "" }
}
