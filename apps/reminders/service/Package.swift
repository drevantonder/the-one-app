// swift-tools-version: 6.0
// Reminders' service: a helper app of its own on a Mac, which macOS lets reach the person's reminders (service/mac-helpers.sh).
import PackageDescription

let package = Package(
    name: "reminders",
    platforms: [.macOS("26.0")],
    dependencies: [.package(path: "../../../service/swift")],
    targets: [.executableTarget(name: "reminders", dependencies: [.product(name: "Helper", package: "swift")], path: ".", exclude: ["Package.swift", "Info.plist", "Helper.entitlements"])]
)
