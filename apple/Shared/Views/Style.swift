import SwiftUI
#if canImport(AppKit)
import AppKit
#else
import UIKit
#endif

extension Color {
    /// The brand's accent, lifted a little in dark mode so it keeps its contrast.
    init(brandHex hex: String) {
        let value = UInt32(hex.trimmingCharacters(in: CharacterSet(charactersIn: "#")), radix: 16) ?? 0x808080
        let r = CGFloat((value >> 16) & 0xFF) / 255, g = CGFloat((value >> 8) & 0xFF) / 255, b = CGFloat(value & 0xFF) / 255
        let lift = { (c: CGFloat) in c + (1 - c) * 0.35 }
        #if canImport(AppKit)
        self.init(nsColor: NSColor(name: nil) { appearance in
            appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
                ? NSColor(srgbRed: lift(r), green: lift(g), blue: lift(b), alpha: 1)
                : NSColor(srgbRed: r, green: g, blue: b, alpha: 1)
        })
        #else
        self.init(uiColor: UIColor { traits in
            traits.userInterfaceStyle == .dark
                ? UIColor(red: lift(r), green: lift(g), blue: lift(b), alpha: 1)
                : UIColor(red: r, green: g, blue: b, alpha: 1)
        })
        #endif
    }
}

/// A person's circle: their initials on a colour that stays theirs on every device.
struct Avatar: View {
    var name: String
    var person: String
    var size: CGFloat = 32

    private static let palette: [Color] = [.blue, .indigo, .purple, .pink, .red, .orange, .yellow, .green, .mint, .teal, .cyan, .brown]

    var body: some View {
        let color = Self.palette[Int(person.utf8.reduce(UInt32(5381)) { ($0 &* 33) &+ UInt32($1) } % UInt32(Self.palette.count))]
        Circle()
            .fill(color.gradient)
            .overlay {
                Text(Self.initials(name))
                    .font(.system(size: size * 0.4, weight: .semibold, design: .rounded))
                    .foregroundStyle(.white)
            }
            .frame(width: size, height: size)
            .accessibilityHidden(true)
    }

    static func initials(_ name: String) -> String {
        let base = name.hasSuffix(".local") ? String(name.dropLast(6)) : name
        let words = base.split { !$0.isLetter && !$0.isNumber && $0 != "’" && $0 != "'" }
        let letters = words.prefix(2).compactMap { $0.first.map(String.init) }
        return letters.joined().uppercased()
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

extension Date {
    /// "now", "5 min. ago", "yesterday", or a date once it's a week old.
    var relative: String {
        let seconds = Date.now.timeIntervalSince(self)
        if seconds < 60 { return String(localized: "now") }
        if seconds < 7 * 86400 { return formatted(.relative(presentation: .named, unitsStyle: .abbreviated)) }
        return formatted(date: .abbreviated, time: .omitted)
    }
}
