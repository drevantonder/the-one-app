// swift-tools-version: 6.0
// Contacts' service: a helper app of its own on a Mac, which macOS lets read the person's contacts (service/mac-helpers.sh).
import PackageDescription

let package = Package(
    name: "contacts",
    platforms: [.macOS("26.0")],
    dependencies: [.package(path: "../../../service/swift")],
    targets: [.executableTarget(name: "contacts", dependencies: [.product(name: "Helper", package: "swift")], path: ".", exclude: ["Package.swift", "Info.plist", "Helper.entitlements"])]
)
