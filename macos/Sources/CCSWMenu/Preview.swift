import AppKit
import SwiftUI

/// Offline visual QA: renders supplied fixture data without starting the helper or timers.
@MainActor enum PreviewRenderer {
    static func run(_ app: NSApplication) {
        let args = CommandLine.arguments
        guard let index = args.firstIndex(of: "--render-preview"), args.count >= index + 3 else { return }
        let input = args[index + 1], output = args[index + 2]
        let dark = args.contains("dark"), chinese = args.contains("zh-Hans")
        do {
            let snapshot = try DashboardSnapshot.decode(Data(contentsOf: URL(fileURLWithPath: input)))
            let suite = "ccsw-preview-" + UUID().uuidString
            let defaults = UserDefaults(suiteName: suite)!
            let snapshotData = try Data(contentsOf: URL(fileURLWithPath: input))
            let managementURL = URL(fileURLWithPath: input).deletingLastPathComponent().appendingPathComponent("management.json")
            let managementData = try? Data(contentsOf: managementURL)
            let workspaceData = try? Data(contentsOf: managementURL.deletingLastPathComponent().appendingPathComponent("workspace.json"))
            let ledgerData = try? Data(contentsOf: managementURL.deletingLastPathComponent().appendingPathComponent("ledger.json"))
            let store = MenuStore(defaults: defaults, execute: { args, _ in
                if args.first == "workspace", let workspaceData { return workspaceData }
                if args.first == "usage-detail", let ledgerData { return ledgerData }
                if args.first == "management", let managementData { return managementData }
                return snapshotData
            })
            store.language = chinese ? "zh-Hans" : "en"
            store.snapshot = snapshot
            store.selectedClient = args.contains("codex") ? "codex" : args.contains("pi") ? "pi" : "claude"
            if let managementData { store.management = try decoder().decode(ManagementSnapshot.self, from: managementData) }
            let manager = args.contains("manager"), editor = args.contains("editor")
            let root: AnyView
            if editor, let provider = store.management?.providers.first { root = AnyView(ProviderEditor(store: store, original: provider, initialSection: args.contains("models") ? "models" : "connection")) }
            else if manager { root = AnyView(ManagementPanel(store: store, initialSelection: "studio", initialPage: ["accounts", "usage", "preferences", "proxy"].first(where: { args.contains($0) }) ?? "providers")) }
            else { root = AnyView(MenuPanel(store: store, openSettings: {})) }
            let view = NSHostingView(rootView: root)
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: editor ? 720 : manager ? 900 : 320, height: editor ? 680 : manager ? 660 : 460), styleMask: [.borderless], backing: .buffered, defer: false)
            window.ignoresMouseEvents = true
            window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
            window.isOpaque = false
            window.backgroundColor = .clear
            window.contentView = view
            window.center()
            window.makeKeyAndOrderFront(nil)
            app.activate(ignoringOtherApps: true)
            DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
                view.layoutSubtreeIfNeeded()
                if let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds) {
                    view.cacheDisplay(in: view.bounds, to: bitmap)
                    if let data = bitmap.representation(using: .png, properties: [:]) {
                        try? data.write(to: URL(fileURLWithPath: output))
                    }
                }
                defaults.removePersistentDomain(forName: suite)
                app.stop(nil)
                let event = NSEvent.otherEvent(with: .applicationDefined, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil, subtype: 0, data1: 0, data2: 0)
                if let event { app.postEvent(event, atStart: true) }
            }
            withExtendedLifetime(window) { app.run() }
        } catch { fputs("Could not render fixture.\n", stderr) }
    }
}
