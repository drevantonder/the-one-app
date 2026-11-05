// swift-tools-version:5.3
import PackageDescription

let package = Package(
  name: "tauri-plugin-one-natives",
  platforms: [.iOS(.v15)],
  products: [.library(name: "tauri-plugin-one-natives", type: .static, targets: ["tauri-plugin-one-natives"])],
  dependencies: [.package(name: "Tauri", path: "../.tauri/tauri-api")],
  targets: [.target(name: "tauri-plugin-one-natives", dependencies: [.byName(name: "Tauri")], path: "Sources")]
)
