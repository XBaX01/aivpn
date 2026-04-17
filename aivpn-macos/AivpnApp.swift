import SwiftUI
import UserNotifications

@main
struct AivpnApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) var appDelegate
    @StateObject private var vpnManager = VPNManager()
    @StateObject private var localization = LocalizationManager()

    var body: some Scene {
        Settings {
            EmptyView()
        }
    }
}

class AppDelegate: NSObject, NSApplicationDelegate {
    var statusItem: NSStatusItem?
    var popover: NSPopover?
    var eventMonitor: Any?

    func applicationDidFinishLaunching(_ notification: Notification) {
        // Hide dock icon — menu bar only
        NSApp.setActivationPolicy(.accessory)
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound]) { _, _ in }

        // Create status bar item
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        if let button = statusItem?.button {
            button.image = NSImage(systemSymbolName: "circle", accessibilityDescription: "Disconnected")
            button.action = #selector(togglePopover(_:))
            button.target = self
        }

        // Create popover
        let contentView = ContentView()
            .environmentObject(VPNManager.shared)
            .environmentObject(LocalizationManager.shared)

        popover = NSPopover()
        popover?.contentSize = NSSize(width: 360, height: 440)
        popover?.behavior = .transient
        popover?.contentViewController = NSHostingController(rootView: contentView)

        // Event monitor to close popover on outside click
        eventMonitor = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) { [weak self] _ in
            if let popover = self?.popover, popover.isShown {
                popover.performClose(nil)
            }
        }

        // Check helper daemon on launch
        VPNManager.shared.checkHelperAvailable()
        
        // Start polling for connection status changes
        startStatusPolling()
    }
    
    var statusPollTimer: Timer?
    
    func startStatusPolling() {
        statusPollTimer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in
            self?.checkConnectionStatus()
        }
    }
    
    func checkConnectionStatus() {
        let connected = VPNManager.shared.isConnected
        updateStatusIcon(connected: connected)
    }

    @objc func togglePopover(_ sender: Any?) {
        guard let popover = popover, let button = statusItem?.button else { return }
        if popover.isShown {
            popover.performClose(nil)
        } else {
            popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)
            NSApp.activate(ignoringOtherApps: true)
        }
    }

    func updateStatusIcon(connected: Bool) {
        guard let button = statusItem?.button else { return }
        
        DispatchQueue.main.async {
            // Use template image with white color for better visibility
            let iconName = connected ? "circle.fill" : "circle"
            
            // Create image as template so it renders in menu bar style (white)
            let image = NSImage(systemSymbolName: iconName, accessibilityDescription: connected ? "Connected" : "Disconnected")
            image?.isTemplate = true  // This makes it render as white in menu bar
            
            // Force image update
            button.image = nil
            button.image = image
            
            // Remove contentTintColor to use system menu bar color (white)
            button.contentTintColor = nil
            
            // Update tooltip
            button.toolTip = connected ? "AIVPN: Connected" : "AIVPN: Disconnected"
        }
    }
}
