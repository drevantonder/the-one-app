// swift-tools-version:5.9
// The Mac's own Swift, its menu bar (src/platform/menubar.rs), which build.rs builds and links into the app's Rust as the
// iPhone's is. Its widgets are the iPhone's drawing (iphone/Widget, symlinked into Sources: a package can't reach outside
// its folder), so a widget draws the same on both. Its target is named as the package is, which is how swift-rs finds
// the C names it makes global again for Xcode 27's linker.
import PackageDescription

let package = Package(
  name: "one-native-mac",
  platforms: [.macOS("26.0")],
  products: [.library(name: "one-native-mac", type: .static, targets: ["OneNativeMac"])],
  targets: [.target(name: "OneNativeMac", path: "Sources/OneNativeMac")]
)
