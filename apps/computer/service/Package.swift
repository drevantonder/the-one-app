// swift-tools-version: 6.0
// Computer Use's service: a helper app of its own on a Mac, which macOS lets see the screen and use the keyboard and pointer (service/mac-helpers.sh).
import PackageDescription

let package = Package(
    name: "computer",
    platforms: [.macOS("26.0")],
    dependencies: [.package(path: "../../../service/swift")],
    targets: [.executableTarget(name: "computer", dependencies: [.product(name: "Helper", package: "swift")], path: ".", exclude: ["Package.swift", "Info.plist", "Helper.entitlements"])]
)
