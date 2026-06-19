import SwiftUI

// Manages excluded domains and CIDR routes stored in the App Group UserDefaults
// so the tunnel extension can read them without IPC.
//
// SECURITY NOTE (C-I-3): The domain and route lists are stored in App Group
// UserDefaults (group.com.aivpn.client) and are NOT cryptographically verified.
// Any process in the same App Group can modify these lists. Do not rely on this
// storage for security-sensitive routing decisions.
class SplitTunnelManager: ObservableObject {
    static let shared = SplitTunnelManager()

    @Published var excludedDomains: [String] = []
    @Published var excludedRoutes: [String] = []

    private let suiteName = "group.com.aivpn.client"
    private let domainsKey = "excluded_domains"
    private let routesKey  = "excluded_routes"

    private var defaults: UserDefaults {
        UserDefaults(suiteName: suiteName) ?? .standard
    }

    init() { load() }

    func load() {
        excludedDomains = defaults.stringArray(forKey: domainsKey) ?? []
        excludedRoutes  = defaults.stringArray(forKey: routesKey)  ?? []
    }

    // MARK: - Domain management

    func addDomain(_ domain: String) {
        let d = domain.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !d.isEmpty, !excludedDomains.contains(d) else { return }
        excludedDomains.append(d)
        saveDomains()
    }

    func removeDomain(at offsets: IndexSet) {
        excludedDomains.remove(atOffsets: offsets)
        saveDomains()
    }

    private func saveDomains() {
        defaults.set(excludedDomains, forKey: domainsKey)
    }

    // MARK: - Route management (CIDR strings, e.g. "192.168.1.0/24")

    func addRoute(_ cidr: String) {
        let r = cidr.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !r.isEmpty, !excludedRoutes.contains(r) else { return }
        excludedRoutes.append(r)
        saveRoutes()
    }

    func removeRoute(at offsets: IndexSet) {
        excludedRoutes.remove(atOffsets: offsets)
        saveRoutes()
    }

    private func saveRoutes() {
        defaults.set(excludedRoutes, forKey: routesKey)
    }
}

struct SplitTunnelView: View {
    @StateObject private var mgr = SplitTunnelManager.shared
    @EnvironmentObject private var loc: LocalizationManager
    @Environment(\.dismiss) private var dismiss

    @State private var newDomain: String = ""
    @State private var newRoute: String  = ""

    var body: some View {
        NavigationView {
            List {
                // MARK: Excluded routes (CIDR)
                Section(header: Text(loc.t("split_tunnel_routes"))) {
                    if mgr.excludedRoutes.isEmpty {
                        Text(loc.t("split_tunnel_none"))
                            .foregroundColor(.secondary)
                            .font(.subheadline)
                    } else {
                        ForEach(mgr.excludedRoutes, id: \.self) { route in
                            Text(route)
                                .font(.system(.body, design: .monospaced))
                        }
                        .onDelete { mgr.removeRoute(at: $0) }
                    }
                    HStack {
                        TextField("192.168.1.0/24", text: $newRoute)
                            .autocorrectionDisabled()
                            .autocapitalization(.none)
                            .keyboardType(.numbersAndPunctuation)
                        Button {
                            mgr.addRoute(newRoute)
                            newRoute = ""
                        } label: {
                            Image(systemName: "plus.circle.fill")
                                .foregroundColor(.accentColor)
                        }
                        .disabled(newRoute.trimmingCharacters(in: .whitespaces).isEmpty)
                    }
                }

                // MARK: Excluded domains (split DNS)
                Section(header: Text(loc.t("split_tunnel")),
                        footer: Text("Domain and route lists are stored in App Group UserDefaults and are not cryptographically verified.")
                            .font(.caption2)
                            .foregroundColor(.secondary)) {
                    if mgr.excludedDomains.isEmpty {
                        Text(loc.t("split_tunnel_none"))
                            .foregroundColor(.secondary)
                            .font(.subheadline)
                    } else {
                        ForEach(mgr.excludedDomains, id: \.self) { domain in
                            Text(domain)
                                .font(.system(.body, design: .monospaced))
                        }
                        .onDelete { mgr.removeDomain(at: $0) }
                    }
                    HStack {
                        TextField("example.com", text: $newDomain)
                            .autocorrectionDisabled()
                            .autocapitalization(.none)
                            .keyboardType(.URL)
                        Button {
                            mgr.addDomain(newDomain)
                            newDomain = ""
                        } label: {
                            Image(systemName: "plus.circle.fill")
                                .foregroundColor(.accentColor)
                        }
                        .disabled(newDomain.trimmingCharacters(in: .whitespaces).isEmpty)
                    }
                }
            }
            .navigationTitle(loc.t("split_tunnel"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button(loc.t("save_key")) { dismiss() }
                }
            }
        }
    }
}
