import Foundation

struct ConnectionKey: Identifiable, Codable, Equatable {
    let id: String
    var name: String
    let keyValue: String
    let serverAddress: String?
    let vpnIP: String?
    let canRecord: Bool?
    var mtlsCert: String?

    init(id: String = UUID().uuidString, name: String, keyValue: String, mtlsCert: String? = nil) {
        self.id = id
        self.name = name
        self.keyValue = keyValue.trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "aivpn://", with: "")
        self.mtlsCert = mtlsCert

        var server: String? = nil
        var ip: String? = nil
        var record: Bool? = nil

        var b64 = self.keyValue
            .replacingOccurrences(of: "-", with: "+")
            .replacingOccurrences(of: "_", with: "/")
        let rem = b64.count % 4
        if rem > 0 { b64 += String(repeating: "=", count: 4 - rem) }

        if let data = Data(base64Encoded: b64),
           let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
            server = json["s"] as? String
            ip = json["i"] as? String
            record = json["can_record"] as? Bool
        }

        self.serverAddress = server
        self.vpnIP = ip
        self.canRecord = record
    }

    var fullKey: String { "aivpn://\(keyValue)" }

    var displayName: String {
        if let s = serverAddress { return "\(name) (\(s))" }
        return name
    }

    var isRecordingAdminKey: Bool { canRecord ?? false }
}

// MARK: - Storage (App Group UserDefaults — shared with tunnel extension)

class KeychainStorage: ObservableObject {
    static let shared = KeychainStorage()

    @Published var keys: [ConnectionKey] = []
    @Published var selectedKeyId: String?

    private let suiteName = "group.com.aivpn.client"
    private let keysKey = "saved_connection_keys"
    private let selKey  = "selected_connection_key_id"

    private var defaults: UserDefaults {
        UserDefaults(suiteName: suiteName) ?? .standard
    }

    init() { loadKeys() }

    func loadKeys() {
        if let data = defaults.data(forKey: keysKey),
           let decoded = try? JSONDecoder().decode([ConnectionKey].self, from: data) {
            keys = decoded
        }
        selectedKeyId = defaults.string(forKey: selKey)
        if selectedKeyId != nil && !keys.contains(where: { $0.id == selectedKeyId }) {
            selectedKeyId = nil
        }
        if selectedKeyId == nil, let first = keys.first {
            selectedKeyId = first.id
            defaults.set(selectedKeyId, forKey: selKey)
        }
    }

    private func saveKeys() {
        if let encoded = try? JSONEncoder().encode(keys) {
            defaults.set(encoded, forKey: keysKey)
        }
    }

    func addKey(name: String, keyValue: String, mtlsCert: String? = nil) -> ConnectionKey? {
        let norm = keyValue.trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "aivpn://", with: "")
        if keys.contains(where: { $0.keyValue == norm }) { return nil }
        let k = ConnectionKey(name: name, keyValue: keyValue, mtlsCert: mtlsCert)
        keys.append(k)
        saveKeys()
        if keys.count == 1 { selectKey(id: k.id) }
        return k
    }

    func updateKey(id: String, name: String, keyValue: String, mtlsCert: String? = nil) -> Bool {
        guard let idx = keys.firstIndex(where: { $0.id == id }) else { return false }
        let norm = keyValue.trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "aivpn://", with: "")
        if norm != keys[idx].keyValue,
           keys.contains(where: { $0.id != id && $0.keyValue == norm }) { return false }
        keys[idx] = ConnectionKey(id: id, name: name, keyValue: keyValue, mtlsCert: mtlsCert)
        saveKeys()
        return true
    }

    func deleteKey(id: String) {
        keys.removeAll { $0.id == id }
        saveKeys()
        if selectedKeyId == id {
            selectedKeyId = keys.first?.id
            defaults.set(selectedKeyId, forKey: selKey)
        }
    }

    func selectKey(id: String?) {
        selectedKeyId = id
        defaults.set(id, forKey: selKey)
    }

    var activeKey: ConnectionKey? {
        guard let id = selectedKeyId else { return nil }
        return keys.first(where: { $0.id == id })
    }
}
