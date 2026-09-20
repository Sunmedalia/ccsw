// swift-tools-version: 5.9
import PackageDescription
let package = Package(
    name: "CCSWMenu", platforms: [.macOS(.v13)],
    products: [.executable(name: "CCSWMenu", targets: ["CCSWMenu"])],
    targets: [
        .executableTarget(name: "CCSWMenu"),
        .testTarget(name: "CCSWMenuTests", dependencies: ["CCSWMenu"], resources: [.copy("Fixtures")])
    ])
