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

/// A space's mark: its initial on the brand accent.
struct SpaceMark: View {
    var name: String
    var size: CGFloat

    var body: some View {
        RoundedRectangle(cornerRadius: size * 0.27, style: .continuous)
            .fill(.tint)
            .overlay {
                Text(name.first.map { String($0).uppercased() } ?? "")
                    .font(.system(size: size * 0.5, weight: .semibold, design: .rounded))
                    .foregroundStyle(.white)
            }
            .frame(width: size, height: size)
    }
}
