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

/// A device's symbol, guessed from its name.
func deviceSymbol(_ name: String) -> String {
    let n = name.lowercased()
    if n.contains("iphone") || n.contains("phone") { return "iphone" }
    if n.contains("ipad") { return "ipad" }
    if n.contains("macbook") || n.contains("laptop") { return "laptopcomputer" }
    if n.contains("imac") || n.contains("mac") { return "desktopcomputer" }
    return "server.rack"
}

/// An app's icon: its SF Symbol, or a generic one when it names something else.
func symbol(_ name: String?) -> String {
    #if canImport(AppKit)
    let exists = name.flatMap { NSImage(systemSymbolName: $0, accessibilityDescription: nil) } != nil
    #else
    let exists = name.flatMap { UIImage(systemName: $0) } != nil
    #endif
    return exists ? name! : "app"
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

/// An app's icon tile, as a phone's Home Screen draws one: its own picture (`icon.png` in its folder) on white, or its
/// symbol in white on its colour, the one it declares or else one picked by its name, so apps side by side differ.
struct AppTile: View {
    var icon: String?
    var seed: String
    var color: String? = nil
    var image: String? = nil
    var size: CGFloat

    /// The system's colours, for apps that don't declare one.
    private static let colors = ["#007aff", "#34c759", "#ff9500", "#ff3b30", "#af52de", "#5856d6", "#30b0c7", "#ff2d55",
                                 "#a2845e", "#32ade6", "#00c7be", "#8e8e93"]

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: size * 0.225, style: .continuous)
        Group {
            if let picture = image.flatMap(Self.picture) {
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
                        Image(systemName: symbol(icon))
                            .font(.system(size: size * 0.44, weight: .medium))
                            .foregroundStyle(.white)
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

    /// An app's picture, from the data URL its server gives.
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

/// The system's grouped background, behind a page's grouped containers.
extension View {
    func groupedBackground() -> some View {
        #if os(macOS)
        self
        #else
        background(Color(.systemGroupedBackground))
        #endif
    }

    /// A grouped container, as a list's inset section draws one.
    func groupedContainer() -> some View {
        #if os(macOS)
        background(.fill.quaternary, in: .rect(cornerRadius: 12, style: .continuous))
        #else
        background(Color(.secondarySystemGroupedBackground), in: .rect(cornerRadius: 24, style: .continuous))
        #endif
    }
}

/// A shell icon, as Settings draws one: a white symbol on a system colour.
struct SymbolTile: View {
    var symbol: String
    var color: Color
    var size: CGFloat

    var body: some View {
        Image(systemName: symbol)
            .font(.system(size: size * 0.46, weight: .medium))
            .foregroundStyle(.white)
            .frame(width: size, height: size)
            .background(color.gradient, in: .rect(cornerRadius: size * 0.225, style: .continuous))
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
