import SwiftUI

// Manages excluded domains stored in the App Group UserDefaults so the tunnel
// extension can read them without IPC.
class SplitTunnelManager: ObservableObject {
    static let shared = SplitTunnelManager()

    @Published var excludedDomains: [String] = []

    private let suiteName = "group.com.aivpn.client"
    private let key = "excluded_domains"

    private var defaults: UserDefaults {
        UserDefaults(suiteName: suiteName) ?? .standard
    }

    init() { load() }

    func load() {
        excludedDomains = defaults.stringArray(forKey: key) ?? []
    }

    func add(_ domain: String) {
        let d = domain.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !d.isEmpty, !excludedDomains.contains(d) else { return }
        excludedDomains.append(d)
        save()
    }

    func remove(at offsets: IndexSet) {
        excludedDomains.remove(atOffsets: offsets)
        save()
    }

    private func save() {
        defaults.set(excludedDomains, forKey: key)
    }
}

struct SplitTunnelView: View {
    @StateObject private var mgr = SplitTunnelManager.shared
    @EnvironmentObject private var loc: LocalizationManager
    @Environment(\.dismiss) private var dismiss

    @State private var newDomain: String = ""

    var body: some View {
        NavigationView {
            List {
                Section(header: Text(loc.t("split_tunnel"))) {
                    if mgr.excludedDomains.isEmpty {
                        Text(loc.t("split_tunnel_none"))
                            .foregroundColor(.secondary)
                            .font(.subheadline)
                    } else {
                        ForEach(mgr.excludedDomains, id: \.self) { domain in
                            Text(domain)
                                .font(.system(.body, design: .monospaced))
                        }
                        .onDelete { mgr.remove(at: $0) }
                    }
                }

                Section {
                    HStack {
                        TextField("example.com", text: $newDomain)
                            .autocorrectionDisabled()
                            .autocapitalization(.none)
                            .keyboardType(.URL)
                        Button {
                            mgr.add(newDomain)
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
