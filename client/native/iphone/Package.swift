// swift-tools-version:5.9
// The iPhone's Swift, a Tauri plugin of the app's own (src/iphone.rs), which build.rs builds and links into the app's
// Rust, as Tauri's own plugins are, against Tauri's Swift (.tauri/tauri-api, which build.rs copies here).
import PackageDescription

let package = Package(
  name: "one-native-iphone",
  platforms: [.iOS("26.0")],
  products: [.library(name: "one-native-iphone", type: .static, targets: ["OneNative"])],
  dependencies: [.package(name: "Tauri", path: ".tauri/tauri-api")],
  targets: [.target(name: "OneNative", dependencies: [.byName(name: "Tauri")], path: "Sources/OneNative")]
)
