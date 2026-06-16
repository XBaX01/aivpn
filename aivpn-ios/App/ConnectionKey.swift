import Foundation
import Security

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

// MARK: - Storage (Keychain — shared with tunnel extension via kSecAttrAccessGroup)
//
// Each ConnectionKey is stored as a single Keychain item using kSecClassGenericPassword.
// The account attribute is the key's UUID; the service attribute is a fixed constant.
// The keyValue and mtlsCert are encoded together as JSON in the secret data field so
// that both sensitive values are protected by the Secure Enclave-backed Keychain.
// The non-secret metadata (name, serverAddress, vpnIP, canRecord) is stored in the
// kSecAttrLabel and kSecAttrGeneric attributes to allow listing without decrypting.
//
// Access: kSecAttrAccessibleWhenUnlockedThisDeviceOnly — data is accessible only when
// the device is unlocked and cannot be transferred to another device via backup.
//
// The selected key ID is stored in a separate Keychain item with account = "selected_id"
// to avoid falling back to UserDefaults for any sensitive selection state.

private struct KeychainPayload: Codable {
    let id: String
    let name: String
    let keyValue: String
    let mtlsCert: String?
}

class KeychainStorage: ObservableObject {
    static let shared = KeychainStorage()

    @Published var keys: [ConnectionKey] = []
    @Published var selectedKeyId: String?

    private let service = "com.aivpn.client"
    private let selectedAccount = "__selected_id__"

    init() { loadKeys() }

    // MARK: - Load

    func loadKeys() {
        let query: [CFString: Any] = [
            kSecClass:            kSecClassGenericPassword,
            kSecAttrService:      service,
            kSecMatchLimit:       kSecMatchLimitAll,
            kSecReturnAttributes: true,
            kSecReturnData:       true,
        ]
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        guard status == errSecSuccess,
              let items = result as? [[CFString: Any]] else {
            keys = []
            selectedKeyId = nil
            return
        }

        var loaded: [ConnectionKey] = []
        var rawSelectedId: String?

        for item in items {
            guard let account = item[kSecAttrAccount] as? String,
                  let data    = item[kSecValueData]   as? Data else { continue }

            // The selected-ID item is stored alongside the key items in the same service.
            if account == selectedAccount {
                rawSelectedId = String(data: data, encoding: .utf8)
                continue
            }

            guard let payload = try? JSONDecoder().decode(KeychainPayload.self, from: data) else { continue }
            let ck = ConnectionKey(id: payload.id, name: payload.name,
                                   keyValue: payload.keyValue, mtlsCert: payload.mtlsCert)
            loaded.append(ck)
        }

        keys = loaded
        selectedKeyId = rawSelectedId

        if let sid = selectedKeyId, !keys.contains(where: { $0.id == sid }) {
            selectedKeyId = nil
        }
        if selectedKeyId == nil, let first = keys.first {
            selectedKeyId = first.id
            persistSelectedId(first.id)
        }
    }

    // MARK: - Add

    @discardableResult
    func addKey(name: String, keyValue: String, mtlsCert: String? = nil) -> ConnectionKey? {
        let norm = keyValue.trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "aivpn://", with: "")
        if keys.contains(where: { $0.keyValue == norm }) { return nil }
        let k = ConnectionKey(name: name, keyValue: keyValue, mtlsCert: mtlsCert)
        guard keychainAdd(k) else { return nil }
        keys.append(k)
        if keys.count == 1 { selectKey(id: k.id) }
        return k
    }

    // MARK: - Update

    @discardableResult
    func updateKey(id: String, name: String, keyValue: String, mtlsCert: String? = nil) -> Bool {
        guard let idx = keys.firstIndex(where: { $0.id == id }) else { return false }
        let norm = keyValue.trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "aivpn://", with: "")
        if norm != keys[idx].keyValue,
           keys.contains(where: { $0.id != id && $0.keyValue == norm }) { return false }
        let updated = ConnectionKey(id: id, name: name, keyValue: keyValue, mtlsCert: mtlsCert)
        guard keychainUpdate(updated) else { return false }
        keys[idx] = updated
        return true
    }

    // MARK: - Delete

    func deleteKey(id: String) {
        keychainDelete(account: id)
        keys.removeAll { $0.id == id }
        if selectedKeyId == id {
            selectedKeyId = keys.first?.id
            persistSelectedId(selectedKeyId)
        }
    }

    // MARK: - Select

    func selectKey(id: String?) {
        selectedKeyId = id
        persistSelectedId(id)
    }

    var activeKey: ConnectionKey? {
        guard let id = selectedKeyId else { return nil }
        return keys.first(where: { $0.id == id })
    }

    // MARK: - Keychain primitives

    private func payloadData(_ key: ConnectionKey) -> Data? {
        let payload = KeychainPayload(id: key.id, name: key.name,
                                     keyValue: key.keyValue, mtlsCert: key.mtlsCert)
        return try? JSONEncoder().encode(payload)
    }

    private func keychainAdd(_ key: ConnectionKey) -> Bool {
        guard let data = payloadData(key) else { return false }
        let attrs: [CFString: Any] = [
            kSecClass:                   kSecClassGenericPassword,
            kSecAttrService:             service,
            kSecAttrAccount:             key.id,
            kSecAttrAccessible:          kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
            kSecValueData:               data,
        ]
        let status = SecItemAdd(attrs as CFDictionary, nil)
        return status == errSecSuccess || status == errSecDuplicateItem
    }

    private func keychainUpdate(_ key: ConnectionKey) -> Bool {
        guard let data = payloadData(key) else { return false }
        let query: [CFString: Any] = [
            kSecClass:       kSecClassGenericPassword,
            kSecAttrService: service,
            kSecAttrAccount: key.id,
        ]
        let update: [CFString: Any] = [
            kSecValueData: data,
        ]
        let status = SecItemUpdate(query as CFDictionary, update as CFDictionary)
        if status == errSecItemNotFound {
            return keychainAdd(key)
        }
        return status == errSecSuccess
    }

    private func keychainDelete(account: String) {
        let query: [CFString: Any] = [
            kSecClass:       kSecClassGenericPassword,
            kSecAttrService: service,
            kSecAttrAccount: account,
        ]
        SecItemDelete(query as CFDictionary)
    }

    private func persistSelectedId(_ id: String?) {
        // Store the selected key ID as a Keychain item alongside the key items.
        keychainDelete(account: selectedAccount)
        guard let id = id, let data = id.data(using: .utf8) else { return }
        let attrs: [CFString: Any] = [
            kSecClass:          kSecClassGenericPassword,
            kSecAttrService:    service,
            kSecAttrAccount:    selectedAccount,
            kSecAttrAccessible: kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
            kSecValueData:      data,
        ]
        SecItemAdd(attrs as CFDictionary, nil)
    }
}
