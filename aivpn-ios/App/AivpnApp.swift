import SwiftUI

@main
struct AivpnApp: App {
    @StateObject private var vpnManager = VPNManager.shared
    @StateObject private var localization = LocalizationManager.shared

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(vpnManager)
                .environmentObject(localization)
        }
    }
}
