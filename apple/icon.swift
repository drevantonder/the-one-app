// Draws the app icon in the brand's accent: swift icon.swift "#3A6E8C" out.icns
import AppKit

let args = CommandLine.arguments
let hex = UInt32(args[1].trimmingCharacters(in: CharacterSet(charactersIn: "#")), radix: 16)!
let out = URL(filePath: args[2])
func channel(_ shift: UInt32) -> CGFloat { CGFloat((hex >> shift) & 0xFF) / 255 }
let accent = NSColor(srgbRed: channel(16), green: channel(8), blue: channel(0), alpha: 1)
let light = accent.blended(withFraction: 0.35, of: .white)!
let dark = accent.blended(withFraction: 0.25, of: .black)!

func draw(_ size: Int) -> Data {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size, bitsPerSample: 8, samplesPerPixel: 4,
                               hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    let s = CGFloat(size) / 1024
    let tile = NSRect(x: 100 * s, y: 100 * s, width: 824 * s, height: 824 * s)
    let path = NSBezierPath(roundedRect: tile, xRadius: 185 * s, yRadius: 185 * s)
    let shadow = NSShadow()
    shadow.shadowColor = .black.withAlphaComponent(0.25)
    shadow.shadowBlurRadius = 24 * s
    shadow.shadowOffset = NSSize(width: 0, height: -10 * s)
    NSGraphicsContext.saveGraphicsState()
    shadow.set()
    NSGradient(starting: light, ending: dark)!.draw(in: path, angle: -90)
    NSGraphicsContext.restoreGraphicsState()

    let config = NSImage.SymbolConfiguration(pointSize: 400 * s, weight: .medium)
        .applying(.init(paletteColors: [.white]))
    let symbol = NSImage(systemSymbolName: "point.3.filled.connected.trianglepath.dotted", accessibilityDescription: nil)!
        .withSymbolConfiguration(config)!
    let fit = min(500 * s / symbol.size.width, 500 * s / symbol.size.height)
    let w = symbol.size.width * fit, h = symbol.size.height * fit
    symbol.draw(in: NSRect(x: (CGFloat(size) - w) / 2, y: (CGFloat(size) - h) / 2, width: w, height: h))
    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])!
}

let set = FileManager.default.temporaryDirectory.appending(path: "AppIcon-\(getpid()).iconset")
try FileManager.default.createDirectory(at: set, withIntermediateDirectories: true)
for base in [16, 32, 128, 256, 512] {
    try draw(base).write(to: set.appending(path: "icon_\(base)x\(base).png"))
    try draw(base * 2).write(to: set.appending(path: "icon_\(base)x\(base)@2x.png"))
}
let iconutil = Process()
iconutil.executableURL = URL(filePath: "/usr/bin/iconutil")
iconutil.arguments = ["-c", "icns", set.path, "-o", out.path]
try iconutil.run()
iconutil.waitUntilExit()
try? FileManager.default.removeItem(at: set)
