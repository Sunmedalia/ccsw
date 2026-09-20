import AppKit
import SwiftUI

@main struct CCSWMenuApp {
    @MainActor static func main() {
        let app = NSApplication.shared
        if CommandLine.arguments.contains("--render-preview") {
            PreviewRenderer.run(app)
            return
        }
        let delegate = AppDelegate()
        app.delegate = delegate
        app.setActivationPolicy(.accessory)
        withExtendedLifetime(delegate) { app.run() }
    }
}
@MainActor final class GlassPanel: NSPanel {
    var dismissPanel: (() -> Void)?
    override var canBecomeKey: Bool { true }
    override func cancelOperation(_ sender: Any?) { dismissPanel?() }
}
@MainActor final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    let store = MenuStore()
    var item: NSStatusItem!
    let panel = GlassPanel(contentRect: NSRect(x: 0, y: 0, width: 320, height: 460), styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
    var outsideMonitor: Any?
    var settingsWindow: NSWindow?
    var managementWindow: NSWindow?
    func applicationDidFinishLaunching(_ notification: Notification) {
        item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        if let button = item.button {
            button.image = NSImage(systemSymbolName: "square.stack.3d.up", accessibilityDescription: "CCSW Menu")
            button.image?.isTemplate = true
            button.toolTip = "CCSW Menu"
            button.target = self
            button.action = #selector(toggle)
        }
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = true
        panel.level = .statusBar
        panel.hidesOnDeactivate = true
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        panel.delegate = self
        panel.dismissPanel = { [weak self] in self?.closePanel() }
        let host = NSHostingController(rootView: MenuPanel(store: store, openSettings: { [weak self] in self?.showSettings() }, openManagement: { [weak self] in self?.showManagement() }))
        panel.contentViewController = host
        host.view.wantsLayer = true
        host.view.layer?.cornerRadius = 12
        host.view.layer?.masksToBounds = true
        outsideMonitor = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) { [weak self] _ in
            self?.closePanel()
        }
        store.start()
    }
    @objc func toggle() {
        if panel.isVisible { closePanel(); return }
        guard let button = item.button, let anchorWindow = button.window else { return }
        let visible = anchorWindow.screen?.visibleFrame ?? NSRect(x: 0, y: 0, width: 1440, height: 900)
        let anchor = anchorWindow.convertToScreen(button.convert(button.bounds, to: nil))
        let height = min(460, visible.height - 24)
        let x = max(visible.minX + 8, min(anchor.midX - 160, visible.maxX - 328))
        panel.setFrame(NSRect(x: x, y: max(visible.minY + 8, anchor.minY - height - 8), width: 320, height: height), display: true)
        panel.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        store.visible = true
        Task { await store.refresh(); await store.refreshActiveIfDue() }
    }
    func closePanel() { panel.orderOut(nil); store.visible = false }
    func windowDidResignKey(_ notification: Notification) {
        if (notification.object as? NSWindow) === panel { closePanel() }
    }
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if !panel.isVisible { toggle() }
        return false
    }
    func applicationWillTerminate(_ notification: Notification) {
        store.stop()
        if let outsideMonitor { NSEvent.removeMonitor(outsideMonitor) }
    }
    func showManagement() {
        closePanel()
        if managementWindow == nil {
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 900, height: 660), styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.minSize = NSSize(width: 760, height: 540)
            window.delegate = self
            let controller = NSHostingController(rootView: ManagementPanel(store: store))
            controller.sizingOptions = []
            window.contentViewController = controller
            window.center()
            managementWindow = window
        }
        managementWindow?.title = store.t("CCSW 配置管理", "CCSW Configuration Manager")
        managementWindow?.makeKeyAndOrderFront(nil)
        store.managementVisible = true
        Task { await store.refreshManagement() }
        NSApp.activate(ignoringOtherApps: true)
    }
    func windowShouldClose(_ sender: NSWindow) -> Bool {
        guard sender === managementWindow, store.preferencesDirty else { return true }
        let alert = NSAlert()
        alert.messageText = store.t("放弃未保存的客户端设置？", "Discard unsaved client settings?")
        alert.addButton(withTitle: store.t("继续编辑", "Keep editing"))
        alert.addButton(withTitle: store.t("放弃", "Discard"))
        return alert.runModal() == .alertSecondButtonReturn
    }
    func windowWillClose(_ notification: Notification) {
        if (notification.object as? NSWindow) === managementWindow { store.managementVisible = false }
    }
    func showSettings() {
        closePanel()
        if settingsWindow == nil {
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 540, height: 640), styleMask: [.titled, .closable], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentViewController = NSHostingController(rootView: SettingsPanel(store: store))
            window.center()
            settingsWindow = window
        }
        settingsWindow?.title = store.t("CCSW Menu 设置", "CCSW Menu Settings")
        settingsWindow?.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }
}
