import Foundation

/// Модель ключа подключения
struct ConnectionKey: Identifiable, Codable, Equatable {
    let id: String  // UUID для идентификации
    var name: String  // Пользовательское имя
    let keyValue: String  // Сам ключ (без aivpn://)
    let serverAddress: String?  // Извлеченный адрес сервера
    let vpnIP: String?  // Извлеченный VPN IP
    let canRecord: Bool?  // Права на запись масок (из поля can_record в ключе)

    init(id: String = UUID().uuidString, name: String, keyValue: String) {
        self.id = id
        self.name = name
        self.keyValue = keyValue.trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "aivpn://", with: "")

        // Извлекаем данные из ключа (URL-safe base64 без padding)
        var server: String? = nil
        var ip: String? = nil
        var record: Bool? = nil

        // Convert URL-safe base64 to standard base64 for Foundation decoding
        var b64 = self.keyValue
            .replacingOccurrences(of: "-", with: "+")
            .replacingOccurrences(of: "_", with: "/")
        // Add padding if needed
        let remainder = b64.count % 4
        if remainder > 0 {
            b64 += String(repeating: "=", count: 4 - remainder)
        }

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
    
    /// Полный ключ с префиксом
    var fullKey: String {
        return "aivpn://\(keyValue)"
    }
    
    /// Отображаемое имя с сервером
    var displayName: String {
        if let server = serverAddress {
            return "\(name) (\(server))"
        }
        return name
    }

    var isRecordingAdminKey: Bool {
        return canRecord ?? false
    }
}

/// Менеджер хранения ключей
class KeychainStorage: ObservableObject {
    static let shared = KeychainStorage()

    @Published var keys: [ConnectionKey] = []
    @Published var selectedKeyId: String?

    private let keychain = KeychainHelper()
    private let keychainKey = "connection_keys_v1"
    private let defaults = UserDefaults.standard
    private let selectedKeyKey = "selected_connection_key_id"

    init() {
        loadKeys()
    }

    /// Загрузить ключи из Keychain (с миграцией из UserDefaults)
    func loadKeys() {
        if let json = keychain.load(key: keychainKey),
           let data = json.data(using: .utf8),
           let decoded = try? JSONDecoder().decode([ConnectionKey].self, from: data) {
            keys = decoded
        } else if let data = defaults.data(forKey: "saved_connection_keys"),
                  let decoded = try? JSONDecoder().decode([ConnectionKey].self, from: data) {
            // Migrate from UserDefaults to Keychain
            keys = decoded
            saveKeys()
            defaults.removeObject(forKey: "saved_connection_keys")
        }

        // selectedKeyId хранится в UserDefaults — это UI-состояние, не секрет
        selectedKeyId = defaults.string(forKey: selectedKeyKey)

        if selectedKeyId != nil && !keys.contains(where: { $0.id == selectedKeyId }) {
            selectedKeyId = nil
        }

        if selectedKeyId == nil && !keys.isEmpty {
            selectedKeyId = keys.first?.id
            defaults.set(selectedKeyId, forKey: selectedKeyKey)
        }
    }

    /// Сохранить ключи в Keychain
    private func saveKeys() {
        if let encoded = try? JSONEncoder().encode(keys),
           let json = String(data: encoded, encoding: .utf8) {
            keychain.save(key: keychainKey, value: json)
        }
    }
    
    /// Добавить новый ключ
    func addKey(name: String, keyValue: String) -> ConnectionKey? {
        // Проверить дубликат по значению ключа
        if keys.contains(where: { $0.keyValue == keyValue.trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "aivpn://", with: "") }) {
            return nil
        }
        
        let newKey = ConnectionKey(name: name, keyValue: keyValue)
        keys.append(newKey)
        saveKeys()
        
        // Если это первый ключ, выбрать его
        if keys.count == 1 {
            selectedKeyId = newKey.id
        }
        
        return newKey
    }
    
    /// Обновить имя ключа
    func updateKeyName(id: String, newName: String) {
        if let index = keys.firstIndex(where: { $0.id == id }) {
            keys[index].name = newName
            saveKeys()
        }
    }

    /// Обновить ключ полностью (имя + keyValue)
    func updateKey(id: String, name: String, keyValue: String) -> Bool {
        guard let index = keys.firstIndex(where: { $0.id == id }) else {
            return false
        }
        
        let normalizedKey = keyValue.trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "aivpn://", with: "")
        
        // Проверить дубликат (если ключ меняем на другой существующий)
        if normalizedKey != keys[index].keyValue &&
           keys.contains(where: { $0.id != id && $0.keyValue == normalizedKey }) {
            return false
        }
        
        // Создать новый struct с обновлёнными данными
        keys[index] = ConnectionKey(id: id, name: name, keyValue: keyValue)
        saveKeys()
        return true
    }
    
    /// Удалить ключ
    func deleteKey(id: String) {
        keys.removeAll { $0.id == id }
        saveKeys()
        
        // Если удалили выбранный, выбрать другой
        if selectedKeyId == id {
            selectedKeyId = keys.first?.id
            defaults.set(selectedKeyId, forKey: selectedKeyKey)
        }
    }

    /// Выбрать ключ
    func selectKey(id: String?) {
        selectedKeyId = id
        defaults.set(id, forKey: selectedKeyKey)
    }
    
    /// Получить выбранный ключ
    var selectedKey: ConnectionKey? {
        guard let id = selectedKeyId,
              let key = keys.first(where: { $0.id == id }) else {
            return nil
        }
        return key
    }
}
