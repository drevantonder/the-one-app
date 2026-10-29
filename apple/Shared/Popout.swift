import Combine
import SwiftUI
import WebKit

// Popouts are live web pages a widget opens, such as a Space's desktop. In
// the app one fills the window, to use; on a Mac it can float over every
// app instead, to watch, laid out one of three ways the user switches
// between. Opened from the menu bar, it floats.

struct PopoutItem: Identifiable {
    let id: String // its title: opening one already shown brings it back
    let url: URL
    let web: WKWebView
    var onClose: (@MainActor () -> Void)?
}

private func webView(_ url: URL) -> WKWebView {
    let config = WKWebViewConfiguration()
    config.mediaTypesRequiringUserActionForPlayback = []
    let web = WKWebView(frame: .zero, configuration: config)
    web.load(URLRequest(url: url))
    return web
}

#if os(macOS)
import AppKit

@MainActor
final class Popouts: ObservableObject {
    static let shared = Popouts()

    enum Layout: String, CaseIterable {
        case separate, grid, focus
        var icon: String {
            switch self {
            case .separate: "rectangle.stack"
            case .grid: "square.grid.2x2"
            case .focus: "rectangle.bottomhalf.inset.filled"
            }
        }
        var name: String {
            switch self {
            case .separate: "Separate windows"
            case .grid: "One window, a grid"
            case .focus: "One big, the rest small"
            }
        }
    }

    @Published private(set) var items: [PopoutItem] = []
    /// The page shown big in the app's window.
    @Published var big: PopoutItem?
    weak var appWindow: NSWindow?
    /// Fires as a page opens, so sheets over the app close for it.
    let opened = PassthroughSubject<Void, Never>()
    @Published var focus: String?
    @Published var driving: Set<String> = [] // clicks and typing go through
    @Published var layout = Layout(rawValue: UserDefaults.standard.string(forKey: "popoutLayout") ?? "") ?? .separate {
        didSet {
            UserDefaults.standard.set(layout.rawValue, forKey: "popoutLayout")
            arrange()
        }
    }
    private var panels: [PopoutPanel] = []

    func open(_ url: URL, _ title: String, _ onClose: (@MainActor () -> Void)?) {
        if let w = appWindow, NSApp.keyWindow === w || NSApp.keyWindow?.sheetParent === w, !items.contains(where: { $0.id == title }) {
            closeBig()
            opened.send()
            withAnimation(.easeOut(duration: 0.2)) {
                big = PopoutItem(id: title, url: url, web: webView(url), onClose: onClose)
            }
            return
        }
        if !items.contains(where: { $0.id == title }) {
            items.append(PopoutItem(id: title, url: url, web: webView(url), onClose: onClose))
        }
        focus = title
        arrange()
    }

    func closeBig() {
        big?.onClose?()
        withAnimation(.easeOut(duration: 0.2)) { big = nil }
    }

    /// Moves the big page out of the app, to float.
    func float() {
        guard let item = big else { return }
        big = nil
        items.append(item)
        focus = item.id
        arrange()
    }

    func close(_ id: String) {
        items.first { $0.id == id }?.onClose?()
        items.removeAll { $0.id == id }
        driving.remove(id)
        if focus == id { focus = items.last?.id }
        arrange()
    }

    func toggleDriving(_ id: String) {
        if driving.contains(id) { driving.remove(id) } else { driving.insert(id) }
    }

    /// Puts the open pages in windows for the layout, keeping each web view.
    private func arrange() {
        let frames = panels.map(\.frame)
        panels.forEach { $0.close() }
        panels = []
        guard !items.isEmpty, let screen = NSScreen.main?.visibleFrame else { return }
        switch layout {
        case .separate:
            for (i, item) in items.enumerated() {
                let size = CGSize(width: 360, height: 225 + 28)
                let origin = CGPoint(x: screen.maxX - size.width - 16, y: screen.minY + 16 + CGFloat(i) * (size.height + 12))
                show(PopoutTile(item: item, popouts: self), frame: i < frames.count && layoutWas == layout ? frames[i] : CGRect(origin: origin, size: size))
            }
        case .grid:
            let cols = Int(ceil(Double(items.count).squareRoot()))
            let rows = Int(ceil(Double(items.count) / Double(cols)))
            let size = CGSize(width: CGFloat(cols) * 320 + 8, height: CGFloat(rows) * 200 + 36)
            show(PopoutGrid(popouts: self), frame: CGRect(origin: CGPoint(x: screen.maxX - size.width - 16, y: screen.minY + 16), size: size))
        case .focus:
            let size = CGSize(width: 520, height: 325 + 28 + (items.count > 1 ? 72 : 0))
            show(PopoutFocus(popouts: self), frame: CGRect(origin: CGPoint(x: screen.maxX - size.width - 16, y: screen.minY + 16), size: size))
        }
        layoutWas = layout
    }
    private var layoutWas: Layout?

    private func show(_ view: some View, frame: CGRect) {
        let panel = PopoutPanel(contentRect: frame, styleMask: [.borderless, .resizable, .nonactivatingPanel], backing: .buffered, defer: false)
        panel.level = .floating
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        panel.isMovableByWindowBackground = true
        panel.backgroundColor = .clear
        panel.hasShadow = true
        panel.contentView = NSHostingView(rootView: view.clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous)))
        panel.orderFrontRegardless()
        panels.append(panel)
    }
}

/// Shows the big page over the app's window.
struct PopoutCover: ViewModifier {
    @ObservedObject var popouts = Popouts.shared

    func body(content: Content) -> some View {
        content.overlay {
            if let item = popouts.big {
                VStack(spacing: 0) {
                    HStack(spacing: 10) {
                        Text(item.id).font(.headline).lineLimit(1)
                        Spacer()
                        Button("Picture in Picture", systemImage: "pip.enter") { popouts.float() }
                        Button("Done") { popouts.closeBig() }.keyboardShortcut(.cancelAction)
                    }
                    .padding(.horizontal, 14).frame(height: 44).background(.bar)
                    WebHost(web: item.web)
                }
                .background(.background)
                .transition(.opacity.combined(with: .scale(scale: 0.98)))
            }
        }
    }
}

/// A borderless panel that can still take typing, for taking over.
final class PopoutPanel: NSPanel {
    override var canBecomeKey: Bool { true }
}

struct WebHost: NSViewRepresentable {
    let web: WKWebView
    func makeNSView(context: Context) -> WKWebView { web }
    func updateNSView(_ view: WKWebView, context: Context) {}
}

/// One page: watched until taken over, with its controls on hover.
struct PopoutPane: View {
    let item: PopoutItem
    @ObservedObject var popouts: Popouts
    var bar = true // its own controls on hover; off where a header has them
    @State private var hover = false

    var body: some View {
        let driving = popouts.driving.contains(item.id)
        ZStack(alignment: .top) {
            WebHost(web: item.web)
            if !driving {
                Color.clear.contentShape(Rectangle()) // watching: clicks don't reach the page
            }
            if bar && (hover || driving) {
                HStack(spacing: 8) {
                    Text(item.id).font(.system(size: 11, weight: .semibold)).lineLimit(1)
                    Spacer(minLength: 4)
                    PaneControls(item: item, popouts: popouts)
                }
                .padding(.horizontal, 8).padding(.vertical, 5)
                .background(.ultraThinMaterial)
            }
        }
        .overlay { if driving { RoundedRectangle(cornerRadius: 10).strokeBorder(Color.accentColor, lineWidth: 2) } }
        .onHover { hover = $0 }
    }
}

struct PaneControls: View {
    let item: PopoutItem
    @ObservedObject var popouts: Popouts

    var body: some View {
        let driving = popouts.driving.contains(item.id)
        Button(driving ? "Give back" : "Take over") { popouts.toggleDriving(item.id) }
            .buttonStyle(.borderedProminent).controlSize(.small)
        Button { popouts.close(item.id) } label: { Image(systemName: "xmark") }.buttonStyle(.borderless)
    }
}

struct LayoutSwitch: View {
    @ObservedObject var popouts: Popouts

    var body: some View {
        HStack(spacing: 2) {
            ForEach(Popouts.Layout.allCases, id: \.self) { l in
                Button { popouts.layout = l } label: {
                    Image(systemName: l.icon).foregroundStyle(popouts.layout == l ? Color.accentColor : .secondary)
                }
                .buttonStyle(.borderless).help(l.name)
            }
        }
    }
}

struct PopoutTile: View {
    let item: PopoutItem
    @ObservedObject var popouts: Popouts

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Text(item.id).font(.system(size: 11, weight: .semibold)).lineLimit(1)
                Spacer()
                LayoutSwitch(popouts: popouts)
                PaneControls(item: item, popouts: popouts)
            }
            .padding(.horizontal, 10).frame(height: 30).background(.bar)
            PopoutPane(item: item, popouts: popouts, bar: false)
        }
    }
}

struct PopoutGrid: View {
    @ObservedObject var popouts: Popouts

    var body: some View {
        VStack(spacing: 0) {
            HStack { Spacer(); LayoutSwitch(popouts: popouts) }.padding(.horizontal, 10).frame(height: 30).background(.bar)
            GeometryReader { geo in
                let cols = max(1, Int(ceil(Double(popouts.items.count).squareRoot())))
                LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 4), count: cols), spacing: 4) {
                    ForEach(popouts.items) { item in
                        PopoutPane(item: item, popouts: popouts)
                            .frame(height: (geo.size.height - 4 * CGFloat(rows(cols) - 1)) / CGFloat(rows(cols)))
                            .clipShape(RoundedRectangle(cornerRadius: 6))
                    }
                }
            }
            .padding(4)
        }
        .background(.background)
    }

    private func rows(_ cols: Int) -> Int { max(1, Int(ceil(Double(popouts.items.count) / Double(cols)))) }
}

struct PopoutFocus: View {
    @ObservedObject var popouts: Popouts

    var body: some View {
        let big = popouts.items.first { $0.id == popouts.focus } ?? popouts.items.first
        VStack(spacing: 4) {
            HStack { Spacer(); LayoutSwitch(popouts: popouts) }.padding(.horizontal, 10).frame(height: 30).background(.bar)
            if let big { PopoutPane(item: big, popouts: popouts) }
            let rest = popouts.items.filter { $0.id != big?.id }
            if !rest.isEmpty {
                HStack(spacing: 4) {
                    ForEach(rest) { item in
                        Button { popouts.focus = item.id } label: {
                            Text(item.id).font(.system(size: 10)).lineLimit(2).frame(width: 110, height: 64)
                                .background(.background.secondary, in: RoundedRectangle(cornerRadius: 6))
                        }
                        .buttonStyle(.plain)
                    }
                    Spacer()
                }
                .padding([.horizontal, .bottom], 4)
            }
        }
        .background(.background)
    }
}

#else

@MainActor
final class Popouts: ObservableObject {
    static let shared = Popouts()
    @Published var shown: PopoutItem?
    /// Fires as a page opens, so sheets over the app close for it.
    let opened = PassthroughSubject<Void, Never>()

    func open(_ url: URL, _ title: String, _ onClose: (@MainActor () -> Void)?) {
        let item = PopoutItem(id: title, url: url, web: webView(url), onClose: onClose)
        opened.send()
        // A sheet closing for it has to be gone before the page can cover the app.
        Task {
            try? await Task.sleep(for: .milliseconds(450))
            shown = item
        }
    }
}

struct WebHost: UIViewRepresentable {
    let web: WKWebView
    func makeUIView(context: Context) -> WKWebView { web }
    func updateUIView(_ view: WKWebView, context: Context) {}
}

/// Shows the page a widget opened over the whole screen.
struct PopoutCover: ViewModifier {
    @ObservedObject var popouts = Popouts.shared
    @State private var closed: (@MainActor () -> Void)?

    func body(content: Content) -> some View {
        content.fullScreenCover(item: $popouts.shown, onDismiss: { closed?(); closed = nil }) { item in
            NavigationStack {
                WebHost(web: item.web).ignoresSafeArea(edges: .bottom)
                    .navigationTitle(item.id).navigationBarTitleDisplayMode(.inline)
                    .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { closed = item.onClose; popouts.shown = nil } } }
            }
        }
    }
}

#endif
