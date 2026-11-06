// swift-tools-version: 6.0
// iMessage's service: a helper app of its own on a Mac, which macOS lets read Messages and send through it (service/mac-helpers.sh).
import PackageDescription

let package = Package(
    name: "imessage",
    platforms: [.macOS("26.0")],
    dependencies: [.package(path: "../../../service/swift")],
    targets: [.executableTarget(name: "imessage", dependencies: [.product(name: "Helper", package: "swift")], path: ".", exclude: ["Package.swift", "Info.plist", "Helper.entitlements"])]
)
