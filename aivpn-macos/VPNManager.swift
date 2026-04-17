import Foundation
import Combine
import UserNotifications

// MARK: - Helper Protocol Types

struct HelperRequest: Codable {
    let action: String
    let key: String?
    let fullTunnel: Bool?
    let binaryPath: String?
    let service: String?
}

struct HelperResponse: Codable {
    let status: String
    let message: String
    let connected: Bool?
    let pid: Int?
    let version: String?
    let log: String?
}

struct RecordingInfoSnapshot: Codable {
    let can_record: Bool?
    let state: String
    let service: String?
    let message: String?
    let mask_id: String?
    let confidence: Float?
    let updated_at_ms: UInt64
}

private let recordingStatusPaths = [
    "/var/run/aivpn/recording.status",
    "/tmp/aivpn-recording.status",
]

private func currentTimestampMs() -> UInt64 {
    UInt64(Date().timeIntervalSince1970 * 1000)
}

enum MaskRecordingState: Equatable {
    case idle
    case starting(service: String)
    case recording(service: String)
    case stopping(service: String)
    case analyzing(service: String)
    case success(service: String, maskId: String?)
    case failed(service: String, reason: String)
}

struct RecordingResultSummary: Equatable {
    let succeeded: Bool
    let title: String
    let details: String
    let updatedAtMs: UInt64
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
    @Published var isCheckingHelper: Bool = true
    @Published var helperVersion: String = ""
    @Published var recordingState: MaskRecordingState = .idle
    @Published var canRecordMasks: Bool = false
    @Published var recordingCapabilityKnown: Bool = false
    @Published var lastRecordingResult: RecordingResultSummary?
    
    // Поддержка списка ключей
    @Published var selectedKeyId: String?
    var keys: [ConnectionKey] {
        get { KeychainStorage.shared.keys }
    }

    private var statusPollTimer: Timer?
    private var trafficTimer: Timer?
    private var minimumRecordingStatusTimestamp: UInt64 = 0
    private var lastRecordingNotificationTimestamp: UInt64 = 0

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
                _ = KeychainStorage.shared.addKey(name: "Default", keyValue: keyValue)
                selectedKeyId = KeychainStorage.shared.selectedKeyId
            }
            savedKey = keyValue
        }

        // Check helper availability after a short delay
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { [weak self] in
            self?.checkHelperAvailable()
        }
    }

    private func helperClientBinaryPath() -> String? {
        let bundledBinary = Bundle.main.bundlePath + "/Contents/Resources/aivpn-client"
        return FileManager.default.isExecutableFile(atPath: bundledBinary) ? bundledBinary : nil
    }

    private func runBundledClientCommand(_ args: [String], completion: ((Bool, String) -> Void)? = nil) {
        guard let binaryPath = helperClientBinaryPath() else {
            completion?(false, "Bundled aivpn-client not found")
            return
        }

        DispatchQueue.global(qos: .userInitiated).async {
            let task = Process()
            task.executableURL = URL(fileURLWithPath: binaryPath)
            task.arguments = args

            let outputPipe = Pipe()
            task.standardOutput = outputPipe
            task.standardError = outputPipe

            do {
                try task.run()
                task.waitUntilExit()
                let outputData = outputPipe.fileHandleForReading.readDataToEndOfFile()
                let output = String(data: outputData, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
                DispatchQueue.main.async {
                    completion?(task.terminationStatus == 0, output)
                }
            } catch {
                DispatchQueue.main.async {
                    completion?(false, error.localizedDescription)
                }
            }
        }
    }

    private func loadRecordingInfoFromDisk() -> RecordingInfoSnapshot? {
        for path in recordingStatusPaths {
            guard let data = try? Data(contentsOf: URL(fileURLWithPath: path)) else {
                continue
            }
            if let snapshot = try? JSONDecoder().decode(RecordingInfoSnapshot.self, from: data) {
                return snapshot
            }
        }
        return nil
    }

    private func requestRecordingStatusRefresh() {
        runBundledClientCommand(["record", "status"], completion: nil)
    }

    func clearRecordingResult() {
        lastRecordingResult = nil
    }

    private func postRecordingResultNotification(title: String, body: String, updatedAtMs: UInt64) {
        guard updatedAtMs > lastRecordingNotificationTimestamp else {
            return
        }

        lastRecordingNotificationTimestamp = updatedAtMs

        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        content.sound = .default

        let request = UNNotificationRequest(
            identifier: "aivpn.recording.\(updatedAtMs)",
            content: content,
            trigger: nil
        )

        UNUserNotificationCenter.current().add(request)
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
        DispatchQueue.global(qos: .userInitiated).async {
            let fd = socket(AF_UNIX, SOCK_STREAM, 0)
            guard fd >= 0 else {
                DispatchQueue.main.async {
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
        isCheckingHelper = true
        sendToHelper(HelperRequest(action: "ping", key: nil, fullTunnel: nil, binaryPath: nil, service: nil),
                     timeoutSeconds: 2.0) { [weak self] response in
            guard let self = self else { return }
            self.isCheckingHelper = false
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
        recordingState = .idle
        canRecordMasks = false
        recordingCapabilityKnown = false
        lastRecordingResult = nil
        minimumRecordingStatusTimestamp = currentTimestampMs()

        // Determine binary path — prefer the one bundled in the app
        let binaryPath = helperClientBinaryPath()

        let request = HelperRequest(
            action: "connect",
            key: normalizedKey,
            fullTunnel: fullTunnel,
            binaryPath: binaryPath,
            service: nil
        )

        sendToHelper(request) { [weak self] response in
            guard let self = self else { return }

            if let response = response, response.status == "ok" {
                // Start polling for status changes
                self.startStatusPolling()
                self.requestRecordingStatusRefresh()
            } else {
                self.isConnecting = false
                if let response = response {
                    self.lastError = response.message
                } else {
                    self.lastError = "Helper not responding"
                    self.checkHelperAvailable()
                }
            }
        }
    }

    func disconnect() {
        let request = HelperRequest(action: "disconnect", key: nil, fullTunnel: nil, binaryPath: nil, service: nil)

        sendToHelper(request) { [weak self] _ in
            guard let self = self else { return }
            self.stopStatusPolling()
            self.trafficTimer?.invalidate()
            self.trafficTimer = nil
            self.recordingState = .idle
            self.canRecordMasks = false
            self.recordingCapabilityKnown = false
            self.lastRecordingResult = nil
            self.minimumRecordingStatusTimestamp = 0

            DispatchQueue.main.async {
                self.isConnecting = false
                self.isConnected = false
            }
        }
    }

    func startMaskRecording(serviceName: String) {
        let trimmedService = serviceName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedService.isEmpty else {
            lastError = "Mask service name is required"
            return
        }
        guard isConnected else {
            lastError = "Connect before recording a mask"
            return
        }
        guard selectedKey != nil else {
            lastError = "No connection key selected"
            return
        }
        guard canRecordMasks else {
            lastError = "Server did not grant recording access for this key"
            return
        }

        minimumRecordingStatusTimestamp = currentTimestampMs()
        lastRecordingResult = nil
        recordingState = .starting(service: trimmedService)

        runBundledClientCommand(["record", "start", "--service", trimmedService]) { [weak self] ok, output in
            guard let self = self else { return }
            if !ok {
                self.recordingState = .failed(service: trimmedService, reason: output.isEmpty ? "Failed to start recording" : output)
            }
        }
    }

    func stopMaskRecording() {
        let currentService: String
        switch recordingState {
        case .recording(let service), .starting(let service):
            currentService = service
        default:
            lastError = "No active recording to stop"
            return
        }

        minimumRecordingStatusTimestamp = currentTimestampMs()
        recordingState = .stopping(service: currentService)
        runBundledClientCommand(["record", "stop"]) { [weak self] ok, output in
            guard let self = self else { return }
            if !ok {
                self.recordingState = .failed(service: currentService, reason: output.isEmpty ? "Failed to stop recording" : output)
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
        sendToHelper(HelperRequest(action: "status", key: nil, fullTunnel: nil, binaryPath: nil, service: nil),
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

            if connected || self.isConnected {
                self.refreshRecordingInfo()
            }
        }
    }

    private func refreshRecordingInfo() {
        if let snapshot = loadRecordingInfoFromDisk() {
            let applied = applyRecordingInfo(snapshot)
            if !applied || snapshot.can_record == nil {
                requestRecordingStatusRefresh()
            }
        } else {
            requestRecordingStatusRefresh()
        }
    }

    @discardableResult
    private func applyRecordingInfo(_ snapshot: RecordingInfoSnapshot) -> Bool {
        if snapshot.updated_at_ms < minimumRecordingStatusTimestamp {
            return false
        }

        minimumRecordingStatusTimestamp = snapshot.updated_at_ms
        recordingCapabilityKnown = snapshot.can_record != nil
        canRecordMasks = snapshot.can_record ?? false

        let service = snapshot.service ?? selectedKey?.name ?? "mask"
        switch snapshot.state {
        case "recording":
            recordingState = .recording(service: service)
        case "stopping":
            recordingState = .stopping(service: service)
        case "analyzing":
            recordingState = .analyzing(service: service)
        case "success":
            recordingState = .success(service: service, maskId: snapshot.mask_id)
            let title = "Mask recorded successfully"
            let details: String
            if let maskId = snapshot.mask_id, !maskId.isEmpty {
                details = "Mask was saved. ID: \(maskId)"
            } else {
                details = "Mask was saved successfully."
            }
            lastRecordingResult = RecordingResultSummary(
                succeeded: true,
                title: title,
                details: details,
                updatedAtMs: snapshot.updated_at_ms
            )
            postRecordingResultNotification(title: title, body: details, updatedAtMs: snapshot.updated_at_ms)
        case "failed":
            recordingState = .failed(service: service, reason: snapshot.message ?? "Recording failed")
            let reason = snapshot.message ?? "Recording failed"
            let title = "Mask recording failed"
            let details = "Mask was not saved. \(reason)"
            lastRecordingResult = RecordingResultSummary(
                succeeded: false,
                title: title,
                details: details,
                updatedAtMs: snapshot.updated_at_ms
            )
            postRecordingResultNotification(title: title, body: details, updatedAtMs: snapshot.updated_at_ms)
        default:
            recordingState = .idle
        }

        return true
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
        sendToHelper(HelperRequest(action: "traffic", key: nil, fullTunnel: nil, binaryPath: nil, service: nil),
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
