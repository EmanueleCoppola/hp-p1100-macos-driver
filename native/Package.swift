// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "P1100Utility",
    platforms: [.macOS(.v13)],
    products: [.executable(name: "P1100Utility", targets: ["P1100Utility"])],
    targets: [
        .target(
            name: "P1100SettingsBackend",
            path: "backend",
            exclude: ["main.swift"],
            sources: ["SettingsBackend.swift"]
        ),
        .executableTarget(name: "P1100Utility", dependencies: ["P1100SettingsBackend"])
    ]
)
