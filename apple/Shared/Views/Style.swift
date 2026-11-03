import SwiftUI
#if canImport(AppKit)
import AppKit
#else
import UIKit
#endif

/// An accent as red, green and blue, lifted a little in dark mode so it keeps its contrast there,
/// yet still carries white text.
func accentRGB(_ hex: String, dark: Bool) -> (CGFloat, CGFloat, CGFloat) {
    let value = UInt32(hex.trimmingCharacters(in: CharacterSet(charactersIn: "#")), radix: 16) ?? 0x808080
    let rgb = [16, 8, 0].map { CGFloat((value >> $0) & 0xFF) / 255 }.map { dark ? $0 + (1 - $0) * 0.2 : $0 }
    return (rgb[0], rgb[1], rgb[2])
}

/// The same, as CSS.
func accentCSS(_ hex: String, dark: Bool) -> String {
    let (r, g, b) = accentRGB(hex, dark: dark)
    return "rgb(\(Int(r * 255)), \(Int(g * 255)), \(Int(b * 255)))"
}

extension Color {
    init(accentHex hex: String) {
        #if canImport(AppKit)
        self.init(nsColor: NSColor(name: nil) { appearance in
            let (r, g, b) = accentRGB(hex, dark: appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua)
            return NSColor(srgbRed: r, green: g, blue: b, alpha: 1)
        })
        #else
        self.init(uiColor: UIColor { traits in
            let (r, g, b) = accentRGB(hex, dark: traits.userInterfaceStyle == .dark)
            return UIColor(red: r, green: g, blue: b, alpha: 1)
        })
        #endif
    }
}

/// Phosphor's icons (vendor/phosphor), which apps name for their tiles, screens and widgets, so every shell draws the
/// same ones: each a path on a 256-point grid, drawn filled. `git-branch` is the regular one, `git-branch-fill` filled.
enum Icons {
    static let paths: [String: String] = Bundle.main.url(forResource: "icons", withExtension: "json")
        .flatMap { try? Data(contentsOf: $0) }
        .flatMap { try? JSONDecoder().decode([String: String].self, from: $0) } ?? [:]

    static func shape(_ name: String?) -> SVGShape? {
        name.flatMap { paths[$0] }.map { SVGShape($0, box: 256) }
    }
}

/// A named icon, in the foreground style, `size` points square; nothing if there's none by that name.
struct Glyph: View {
    var name: String?
    var size: CGFloat = 17

    var body: some View {
        if let shape = Icons.shape(name) {
            shape.fill(.foreground).frame(width: size, height: size)
        }
    }
}

func displayName(_ name: String) -> String {
    name.hasSuffix(".local") ? String(name.dropLast(6)) : name
}

/// The app's name before the node tells us its config.
var appName: String {
    Bundle.main.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String ?? ""
}

/// A steady number for a name, the same on every device.
func steadyHash(_ text: String) -> Int {
    text.unicodeScalars.reduce(5381) { ($0 &* 33 &+ Int($1.value)) & 0x7FFF_FFFF }
}

/// An app's icon tile, as a phone's Home Screen draws one: a finished app icon edge to edge in its own shape, a logo
/// (`icon.png` in its folder, a site's favicon) on white, or its icon filled (else its first letter) in white on its
/// colour, the one it declares or else one picked by its name, so apps side by side differ.
struct AppTile: View {
    var icon: String?
    var seed: String
    var color: String? = nil
    var image: String? = nil
    /// `icon` when `image` is a finished app icon (a Mac app's, a site's apple-touch-icon); else it's a logo.
    var imageKind: String? = nil
    var size: CGFloat

    /// The system's colours, for apps that don't declare one.
    private static let colors = ["#007aff", "#34c759", "#ff9500", "#ff3b30", "#af52de", "#5856d6", "#30b0c7", "#ff2d55",
                                 "#a2845e", "#32ade6", "#00c7be", "#8e8e93"]

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: size * 0.225, style: .continuous)
        Group {
            if let picture = image.flatMap(Self.picture), imageKind == "icon" {
                // A finished app icon is already its own tile: no plate, no border, just the shape it was made for.
                picture.image.resizable().interpolation(.high).aspectRatio(contentMode: .fill)
                    .clipShape(shape)
            } else if let picture = image.flatMap(Self.picture) {
                // A logo with clear corners (a site's icon) sits inset on white, as one drawn on white would.
                shape.fill(.white)
                    .overlay {
                        picture.image.resizable().interpolation(.high).aspectRatio(contentMode: picture.clear ? .fit : .fill)
                            .padding(picture.clear ? size * 0.16 : 0)
                    }
                    .clipShape(shape)
                    .overlay { shape.strokeBorder(.black.opacity(0.1), lineWidth: 0.5) }
            } else {
                let (r, g, b) = accentRGB(color ?? Self.colors[steadyHash(seed) % Self.colors.count], dark: false)
                shape.fill(Color(red: r, green: g, blue: b).gradient)
                    .overlay {
                        Group {
                            if let glyph = Icons.shape(icon.map { "\($0)-fill" }) {
                                glyph.fill(.white).frame(width: size * 0.5, height: size * 0.5)
                            } else {
                                Text(String(seed.prefix(1)).uppercased())
                                    .font(.system(size: size * 0.42, weight: .semibold))
                                    .foregroundStyle(.white)
                            }
                        }
                        .shadow(color: .black.opacity(0.15), radius: 1, y: 1)
                    }
                    .overlay { shape.strokeBorder(.white.opacity(0.18), lineWidth: 0.5) }
            }
        }
        .frame(width: size, height: size)
        .shadow(color: .black.opacity(0.12), radius: size * 0.08, y: size * 0.04)
    }

    /// An app's picture, and whether its corners are clear.
    private final class Picture {
        let image: Image
        let clear: Bool
        init(_ image: Image, clear: Bool) { self.image = image; self.clear = clear }
    }

    nonisolated(unsafe) private static let pictures = NSCache<NSString, Picture>()

    /// An app's picture, from the data URL its device gives.
    private static func picture(_ url: String) -> Picture? {
        if let picture = pictures.object(forKey: url as NSString) { return picture }
        guard let comma = url.firstIndex(of: ","), let data = Data(base64Encoded: String(url[url.index(after: comma)...])),
              let image = PlatformImage(data: data) else { return nil }
        #if canImport(AppKit)
        let picture = Picture(Image(nsImage: image), clear: image.cgImage(forProposedRect: nil, context: nil, hints: nil).map(clearCorners) ?? false)
        #else
        let picture = Picture(Image(uiImage: image), clear: image.cgImage.map(clearCorners) ?? false)
        #endif
        pictures.setObject(picture, forKey: url as NSString)
        return picture
    }

    /// Whether any of a picture's corners is see-through.
    private static func clearCorners(_ image: CGImage) -> Bool {
        var pixels = [UInt8](repeating: 0, count: 8 * 8 * 4)
        let drawn = pixels.withUnsafeMutableBytes { bytes -> Bool in
            guard let context = CGContext(data: bytes.baseAddress, width: 8, height: 8, bitsPerComponent: 8, bytesPerRow: 32,
                                          space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return false }
            context.interpolationQuality = .medium
            context.draw(image, in: CGRect(x: 0, y: 0, width: 8, height: 8))
            return true
        }
        return drawn && [0, 7, 56, 63].contains { pixels[$0 * 4 + 3] < 128 }
    }
}

#if canImport(AppKit)
typealias PlatformImage = NSImage
#else
typealias PlatformImage = UIImage
#endif

/// The kit's colours (src/screen/kit.css), light then dark: tests/contracts.rs holds them to it.
enum Kit {
    static let bg = Color(light: 0xffffff, dark: 0x141414)
    static let card = Color(light: 0xf4f4f4, dark: 0x222222)
    static let raised = Color(light: 0xffffff, dark: 0x3a3a3a)
    static let fill = Color(light: 0xececec, dark: 0x2b2b2b)
    static let line = Color(light: 0xe6e6e6, dark: 0x2f2f2f)
    static let text = Color(light: 0x111111, dark: 0xf2f2f2)
    static let muted = Color(light: 0x6b6b6b, dark: 0x9b9b9b)
    static let faint = Color(light: 0xc8c8c8, dark: 0x4d4d4d)
    static let warn = Color(light: 0xb45309, dark: 0xfbbf24)
    static let danger = Color(light: 0xdc2626, dark: 0xf87171)
    static let badge = Color(light: 0xff3b30, dark: 0xff3b30)
}

extension Color {
    init(light: UInt32, dark: UInt32) {
        func rgb(_ v: UInt32) -> (CGFloat, CGFloat, CGFloat) { (CGFloat(v >> 16 & 0xFF) / 255, CGFloat(v >> 8 & 0xFF) / 255, CGFloat(v & 0xFF) / 255) }
        #if os(macOS)
        self.init(nsColor: NSColor(name: nil) { appearance in
            let (r, g, b) = rgb(appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua ? dark : light)
            return NSColor(srgbRed: r, green: g, blue: b, alpha: 1)
        })
        #else
        self.init(uiColor: UIColor { traits in
            let (r, g, b) = rgb(traits.userInterfaceStyle == .dark ? dark : light)
            return UIColor(red: r, green: g, blue: b, alpha: 1)
        })
        #endif
    }
}

/// The page's background, behind its cards.
extension View {
    func groupedBackground() -> some View {
        #if os(macOS)
        self
        #else
        background(Kit.bg)
        #endif
    }

    /// A card, with rows inside.
    func groupedContainer() -> some View {
        background(Kit.card, in: .rect(cornerRadius: 26, style: .continuous))
    }
}

/// A person: their initials on a colour picked by their id, the same one app screens draw (`one.avatar`).
struct Avatar: View {
    var id: String
    var name: String
    var size: CGFloat = 32

    private static let colors: [UInt32] = [0x5e5ce6, 0xff9f0a, 0x30b0c7, 0xff375f, 0x34c759, 0xaf52de, 0x0a84ff, 0xa2845e]

    var body: some View {
        let h = id.unicodeScalars.reduce(UInt32(0)) { $0 &* 31 &+ UInt32($1.value) }
        let rgb = Self.colors[Int(h % UInt32(Self.colors.count))]
        let initials = name.split(separator: " ").compactMap(\.first).prefix(2).map(String.init).joined().uppercased()
        Circle()
            .fill(Color(red: Double(rgb >> 16 & 0xFF) / 255, green: Double(rgb >> 8 & 0xFF) / 255, blue: Double(rgb & 0xFF) / 255).gradient)
            .overlay {
                Text(initials.isEmpty ? "?" : initials)
                    .font(.system(size: size * 0.4, weight: .semibold, design: .rounded))
                    .foregroundStyle(.white)
            }
            .frame(width: size, height: size)
            .accessibilityHidden(true)
    }
}

/// Light, Dark or Automatic, as chosen in Settings on this device.
enum Appearance: String, CaseIterable, Identifiable {
    case automatic, light, dark

    var id: Self { self }

    var title: String {
        switch self {
        case .automatic: String(localized: "Automatic")
        case .light: String(localized: "Light")
        case .dark: String(localized: "Dark")
        }
    }

    var scheme: ColorScheme? {
        switch self {
        case .automatic: nil
        case .light: .light
        case .dark: .dark
        }
    }
}

extension View {
    /// A navigation title that keeps every word: on a phone, a long one wraps to two lines and shrinks a little,
    /// where the bar would cut it short.
    func wholeTitle(_ title: String) -> some View {
        #if os(iOS)
        navigationTitle(title)
            .toolbar {
                ToolbarItem(placement: .principal) {
                    Text(title)
                        .font(.headline)
                        .lineLimit(2)
                        .minimumScaleFactor(0.8)
                        .multilineTextAlignment(.center)
                        .accessibilityAddTraits(.isHeader)
                }
            }
        #else
        navigationTitle(title)
        #endif
    }
}
