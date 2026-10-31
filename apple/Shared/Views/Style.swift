import SwiftUI
#if canImport(AppKit)
import AppKit
#else
import UIKit
#endif

/// The brand's accent as red, green and blue, lifted a little in dark mode so it keeps its contrast there,
/// yet still carries white text.
func brandRGB(_ hex: String, dark: Bool) -> (CGFloat, CGFloat, CGFloat) {
    let value = UInt32(hex.trimmingCharacters(in: CharacterSet(charactersIn: "#")), radix: 16) ?? 0x808080
    let rgb = [16, 8, 0].map { CGFloat((value >> $0) & 0xFF) / 255 }.map { dark ? $0 + (1 - $0) * 0.2 : $0 }
    return (rgb[0], rgb[1], rgb[2])
}

/// The same, as CSS.
func brandCSS(_ hex: String, dark: Bool) -> String {
    let (r, g, b) = brandRGB(hex, dark: dark)
    return "rgb(\(Int(r * 255)), \(Int(g * 255)), \(Int(b * 255)))"
}

extension Color {
    init(brandHex hex: String) {
        #if canImport(AppKit)
        self.init(nsColor: NSColor(name: nil) { appearance in
            let (r, g, b) = brandRGB(hex, dark: appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua)
            return NSColor(srgbRed: r, green: g, blue: b, alpha: 1)
        })
        #else
        self.init(uiColor: UIColor { traits in
            let (r, g, b) = brandRGB(hex, dark: traits.userInterfaceStyle == .dark)
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

/// The app's name before the node tells us its brand.
var appName: String {
    Bundle.main.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String ?? ""
}

/// The brand's accent, as this build was made with it.
var accentHex: String { Bundle.main.object(forInfoDictionaryKey: "BrandAccent") as? String ?? "" }

/// A colour near the accent: turned `turn` of the way around the colour wheel, with its brightness scaled.
func accentShade(turn: Double, brightness: Double = 1, saturation: Double = 1) -> Color {
    let (r, g, b) = brandRGB(accentHex, dark: false)
    let maxC = max(r, g, b), minC = min(r, g, b), d = maxC - minC
    var h: Double = 0
    if d > 0 {
        h = maxC == r ? (g - b) / d : maxC == g ? 2 + (b - r) / d : 4 + (r - g) / d
        h = (h / 6).truncatingRemainder(dividingBy: 1)
        if h < 0 { h += 1 }
    }
    let s = maxC == 0 ? 0 : d / maxC
    var hue = (h + turn).truncatingRemainder(dividingBy: 1)
    if hue < 0 { hue += 1 }
    return Color(hue: hue, saturation: min(1, s * saturation), brightness: min(1, maxC * brightness))
}

/// A steady number for a name, the same on every device.
func steadyHash(_ text: String) -> Int {
    text.unicodeScalars.reduce(5381) { ($0 &* 33 &+ Int($1.value)) & 0x7FFF_FFFF }
}

/// An app's icon tile: its symbol in white on a gradient near the accent, turned by its name.
struct AppTile: View {
    var symbol: String
    var seed: String
    var size: CGFloat

    var body: some View {
        let turn = Double(steadyHash(seed) % 9 - 4) * 0.028
        RoundedRectangle(cornerRadius: size * 0.225, style: .continuous)
            .fill(LinearGradient(colors: [accentShade(turn: turn, brightness: 1.25, saturation: 0.85), accentShade(turn: turn + 0.02, brightness: 0.9)],
                                 startPoint: .top, endPoint: .bottom))
            .overlay {
                Image(systemName: symbol)
                    .font(.system(size: size * 0.44, weight: .medium))
                    .foregroundStyle(.white)
                    .shadow(color: .black.opacity(0.15), radius: 1, y: 1)
            }
            .overlay {
                RoundedRectangle(cornerRadius: size * 0.225, style: .continuous)
                    .strokeBorder(.white.opacity(0.18), lineWidth: 0.5)
            }
            .frame(width: size, height: size)
            .shadow(color: .black.opacity(0.12), radius: size * 0.08, y: size * 0.04)
    }
}

/// A page's background: a soft gradient of the accent, turned a little for each page so pages tell apart.
struct PageBackground: View {
    var seed: String
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        let turn = seed == "user" ? 0 : Double(steadyHash(seed) % 7 - 3) * 0.03
        let dark = scheme == .dark
        LinearGradient(stops: [
            .init(color: accentShade(turn: turn, brightness: dark ? 0.55 : 1.3, saturation: dark ? 0.9 : 0.45).opacity(dark ? 0.9 : 0.75), location: 0),
            .init(color: accentShade(turn: turn - 0.04, brightness: dark ? 0.35 : 1.4, saturation: dark ? 0.7 : 0.25).opacity(dark ? 0.7 : 0.45), location: 0.45),
            .init(color: accentShade(turn: turn + 0.05, brightness: dark ? 0.25 : 1.4, saturation: 0.3).opacity(dark ? 0.6 : 0.35), location: 1),
        ], startPoint: .topLeading, endPoint: .bottomTrailing)
        .background(dark ? Color.black : Color.white)
        .ignoresSafeArea()
    }
}
