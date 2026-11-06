// swift-tools-version: 6.0
// What every Mac service in Swift (apps/<name>/service/) is built on, as `service/` is for those in Rust: a helper app
// of its own, which holds its own macOS permission, registering with its oned and answering calls.
import PackageDescription

let package = Package(
    name: "Helper",
    platforms: [.macOS("26.0")],
    products: [.library(name: "Helper", targets: ["Helper"])],
    targets: [.target(name: "Helper", path: ".", exclude: ["Package.swift"], sources: ["Helper.swift", "Events.swift"])]
)
