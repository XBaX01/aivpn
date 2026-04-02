import Foundation
import Combine

// MARK: - Helper Protocol Types

struct HelperRequest: Codable {
    let action: String
    let key: String?
    let fullTunnel: Bool?
    let binaryPath: String?
}

struct HelperResponse: Codable {
    let status: String
    let message: String
    let connected: Bool?
    let pid: Int?
    let version: String?
    let log: String?
}

// MARK: - VPNManager

class VPNManager: ObservableObject {
    static let shared = VPNManager()

    @Published var isConnected: Bool = false
    @Published var isConnecting: Bool = false
    @Published var lastError: String?
    @Published var bytesSent: Int64 = 0
    @Published var bytesReceived: Int64 = 0
    @Published var savedKey: String = ""
    @Published var helperAvailable: Bool = false
    @Published var helperVersion: String = ""
    
    // Поддержка списка ключей
    @Published var selectedKeyId: String?
    var keys: [ConnectionKey] {
        get { KeychainStorage.shared.keys }
    }

    private var statusPollTimer: Timer?
    private var trafficTimer: Timer?

    private let socketPath = "/var/run/aivpn/helper.sock"

    // Use UserDefaults instead of Keychain to avoid keychain prompts
    // for ad-hoc signed apps. The key is only useful with the server anyway.
    private let defaults = UserDefaults.standard

    init() {
        // Загрузить ключи из нового хранилища
        KeychainStorage.shared.loadKeys()
        selectedKeyId = KeychainStorage.shared.selectedKeyId
        
        // Для обратной совместимости: если есть старый ключ и нет новых, добавить его
        if let raw = defaults.string(forKey: "connection_key"), !raw.isEmpty {
            let keyValue = raw.trimmingCharacters(in: CharacterSet.whitespacesAndNewlines)
                .replacingOccurrences(of: "aivpn://", with: "")
            if KeychainStorage.shared.keys.isEmpty {
                KeychainStorage.shared.addKey(name: "Default", keyValue: keyValue)
                selectedKeyId = KeychainStorage.shared.selectedKeyId
            }
            savedKey = keyValue
        }

        // Check helper availability after a short delay
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { [weak self] in
            self?.checkHelperAvailable()
        }
    }

    // MARK: - Key Storage (UserDefaults — no keychain prompts)

    private func saveKey(_ key: String) {
        defaults.set(key, forKey: "connection_key")
    }
    
    // MARK: - Key Management
    
    /// Выбрать ключ по ID
    func selectKey(id: String?) {
        selectedKeyId = id
        KeychainStorage.shared.selectKey(id: id)
        
        if let key = KeychainStorage.shared.selectedKey {
            savedKey = key.keyValue
        }
    }
    
    /// Добавить новый ключ
    func addKey(name: String, keyValue: String) -> Bool {
        if let newKey = KeychainStorage.shared.addKey(name: name, keyValue: keyValue) {
            KeychainStorage.shared.selectKey(id: newKey.id)
            selectedKeyId = newKey.id
            savedKey = newKey.keyValue
            return true
        }
        return false
    }
    
    /// Удалить ключ
    func deleteKey(id: String) {
        KeychainStorage.shared.deleteKey(id: id)
        if selectedKeyId == id {
            selectedKeyId = KeychainStorage.shared.selectedKeyId
            savedKey = KeychainStorage.shared.selectedKey?.keyValue ?? ""
        }
    }
    
    /// Обновить имя ключа
    func updateKeyName(id: String, newName: String) {
        KeychainStorage.shared.updateKeyName(id: id, newName: newName)
    }
    
    /// Обновить ключ полностью
    func updateKey(id: String, name: String, keyValue: String) -> Bool {
        let updated = KeychainStorage.shared.updateKey(id: id, name: name, keyValue: keyValue)
        if updated, selectedKeyId == id,
           let key = KeychainStorage.shared.keys.first(where: { $0.id == id }) {
            savedKey = key.keyValue
        }
        return updated
    }

    /// Получить выбранный ключ
    var selectedKey: ConnectionKey? {
        guard let id = selectedKeyId else {
            return nil
        }
        return KeychainStorage.shared.keys.first(where: { $0.id == id })
    }

    // MARK: - Helper Communication

    /// Send a request to the helper daemon via Unix socket with timeout
    private func sendToHelper(_ request: HelperRequest, timeoutSeconds: Double = 3.0,
                              completion: @escaping (HelperResponse?) -> Void) {
        let sockPath = self.socketPath
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let fd = socket(AF_UNIX, SOCK_STREAM, 0)
            guard fd >= 0 else {
                DispatchQueue.main.async {
                    self?.helperAvailable = false
                    completion(nil)
                }
                return
            }

            // Set connection timeout
            var timeout = timeval(tv_sec: Int(timeoutSeconds), tv_usec: 0)
            setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO,
                       &timeout, socklen_t(MemoryLayout<timeval>.size))
            setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO,
                       &timeout, socklen_t(MemoryLayout<timeval>.size))

            // Build sockaddr_un
            var addrBuf = [Int8](repeating: 0, count: 106)
            addrBuf[0] = 0
            addrBuf[1] = Int8(AF_UNIX)
            let pathBytes = Array(sockPath.utf8)
            for (i, byte) in pathBytes.enumerated() where i + 2 < addrBuf.count {
                addrBuf[i + 2] = Int8(bitPattern: byte)
            }

            let connectResult = addrBuf.withUnsafeBufferPointer { ptr in
                Darwin.connect(fd, UnsafeRawPointer(ptr.baseAddress!).assumingMemoryBound(to: sockaddr.self),
                               socklen_t(addrBuf.count))
            }

            guard connectResult == 0 else {
                close(fd)
                DispatchQueue.main.async {
                    self?.helperAvailable = false
                    completion(nil)
                }
                return
            }

            // Send request
            if let requestData = try? JSONEncoder().encode(request),
               let requestStr = String(data: requestData, encoding: .utf8) {
                _ = requestStr.withCString { ptr in
                    write(fd, ptr, Int(strlen(ptr)))
                }
            }

            // Read response
            var buffer = [UInt8](repeating: 0, count: 65536)
            let bytesRead = read(fd, &buffer, buffer.count)
            close(fd)

            guard bytesRead > 0 else {
                DispatchQueue.main.async {
                    completion(nil)
                }
                return
            }

            let data = Data(bytes: buffer, count: bytesRead)
            if let response = try? JSONDecoder().decode(HelperResponse.self, from: data) {
                DispatchQueue.main.async {
                    completion(response)
                }
            } else {
                DispatchQueue.main.async {
                    completion(nil)
                }
            }
        }
    }

    /// Check if the helper daemon is available
    func checkHelperAvailable() {
        sendToHelper(HelperRequest(action: "ping", key: nil, fullTunnel: nil, binaryPath: nil),
                     timeoutSeconds: 2.0) { [weak self] response in
            guard let self = self else { return }
            if let response = response, response.status == "ok" {
                self.helperAvailable = true
                self.helperVersion = response.version ?? ""
                // Check if already connected
                if let connected = response.connected, connected {
                    self.isConnected = true
                    self.startStatusPolling()
                    self.startTrafficMonitor()
                } else if response.connected != nil {
                    // Helper responded with status — start polling to track
                    self.startStatusPolling()
                }
            } else {
                self.helperAvailable = false
            }
        }
    }

    // MARK: - Connect / Disconnect

    func connect(key: String, fullTunnel: Bool = false) {
        guard !isConnecting else { return }

        let normalizedKey = key.trimmingCharacters(in: CharacterSet.whitespacesAndNewlines)
            .replacingOccurrences(of: "aivpn://", with: "")

        savedKey = normalizedKey
        saveKey(normalizedKey)

        isConnecting = true
        lastError = nil
        bytesSent = 0
        bytesReceived = 0

        // Determine binary path — prefer the one bundled in the app
        let bundledBinary = Bundle.main.bundlePath + "/Contents/Resources/aivpn-client"
        let binaryPath = FileManager.default.isExecutableFile(atPath: bundledBinary) ? bundledBinary : nil

        let request = HelperRequest(
            action: "connect",
            key: normalizedKey,
            fullTunnel: fullTunnel,
            binaryPath: binaryPath
        )

        sendToHelper(request) { [weak self] response in
            guard let self = self else { return }

            if let response = response, response.status == "ok" {
                // Start polling for status changes
                self.startStatusPolling()
            } else {
                self.isConnecting = false
                if let response = response {
                    self.lastError = response.message
                } else {
                    self.lastError = "Helper not responding"
                    self.helperAvailable = false
                }
            }
        }
    }

    func disconnect() {
        let request = HelperRequest(action: "disconnect", key: nil, fullTunnel: nil, binaryPath: nil)

        sendToHelper(request) { [weak self] _ in
            guard let self = self else { return }
            self.stopStatusPolling()
            self.trafficTimer?.invalidate()
            self.trafficTimer = nil

            DispatchQueue.main.async {
                self.isConnecting = false
                self.isConnected = false
            }
        }
    }

    // MARK: - Status Polling (replaces log file monitoring)

    private func startStatusPolling() {
        statusPollTimer?.invalidate()
        // Poll every 2 seconds
        statusPollTimer = Timer.scheduledTimer(withTimeInterval: 2.0, repeats: true) { [weak self] _ in
            self?.pollStatus()
        }
    }

    private func stopStatusPolling() {
        statusPollTimer?.invalidate()
        statusPollTimer = nil
    }

    private func pollStatus() {
        sendToHelper(HelperRequest(action: "status", key: nil, fullTunnel: nil, binaryPath: nil),
                     timeoutSeconds: 2.0) { [weak self] response in
            guard let self = self, let response = response else { return }

            guard response.status == "ok" else { return }

            let connected = response.connected ?? false
            let message = response.message

            if connected && !self.isConnected {
                // Transition: connecting → connected
                DispatchQueue.main.async {
                    self.isConnecting = false
                    self.isConnected = true
                    self.lastError = nil
                    self.startTrafficMonitor()
                }
            } else if !connected && self.isConnected {
                // Transition: connected → disconnected
                DispatchQueue.main.async {
                    self.isConnecting = false
                    self.isConnected = false
                    self.lastError = message
                    self.stopStatusPolling()
                    self.trafficTimer?.invalidate()
                    self.trafficTimer = nil
                }
            } else if !connected && self.isConnecting {
                // Still connecting — check if process died (error message)
                // If message contains "exited" or "Failed" or "ERROR", it's a failure
                let lowerMsg = message.lowercased()
                let isFailure = lowerMsg.contains("exited") ||
                                lowerMsg.contains("failed") ||
                                lowerMsg.contains("error") ||
                                lowerMsg.contains("not found")

                if isFailure {
                    DispatchQueue.main.async {
                        self.isConnecting = false
                        self.isConnected = false
                        self.lastError = message
                        self.stopStatusPolling()
                    }
                } else {
                    // Still connecting — update status message for user
                    DispatchQueue.main.async {
                        self.lastError = nil
                    }
                }
            }
        }
    }

    // MARK: - Traffic Monitor

    private func startTrafficMonitor() {
        trafficTimer?.invalidate()
        trafficTimer = Timer.scheduledTimer(withTimeInterval: 1.0, repeats: true) { [weak self] _ in
            self?.updateTrafficStats()
        }
    }
    
    /// Update traffic statistics from helper logs
    private func updateTrafficStats() {
        // Get log from helper and parse traffic stats
        sendToHelper(HelperRequest(action: "traffic", key: nil, fullTunnel: nil, binaryPath: nil),
                     timeoutSeconds: 1.0) { [weak self] response in
            guard let self = self,
                  let response = response,
                  response.status == "ok" else {
                return
            }
            
            // Response message contains "sent:X,received:Y"
            let parts = response.message.components(separatedBy: ",")
            for part in parts {
                let kv = part.components(separatedBy: ":")
                if kv.count == 2 {
                    if let value = Int64(kv[1]) {
                        if kv[0] == "sent" {
                            DispatchQueue.main.async {
                                self.bytesSent = value
                            }
                        } else if kv[0] == "received" {
                            DispatchQueue.main.async {
                                self.bytesReceived = value
                            }
                        }
                    }
                }
            }
        }
    }

    deinit {
        disconnect()
    }
}
