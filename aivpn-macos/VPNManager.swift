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
    let mtlsCertPath: String?
    let excludeRoutes: String?
    let adaptiveMode: Bool?
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
    private var connectGeneration: Int = 0
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

    @Published var isProxyMode: Bool = false

    private var statusPollTimer: Timer?
    private var trafficTimer: Timer?
    private var minimumRecordingStatusTimestamp: UInt64 = 0
    private var lastRecordingNotificationTimestamp: UInt64 = 0

    private var proxyProcess: Process?
    private var proxyPollTimer: Timer?
    private let proxyLogPath = "/tmp/aivpn-proxy.log"

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
    func addKey(name: String, keyValue: String, mtlsCertPath: String? = nil) -> Bool {
        if let newKey = KeychainStorage.shared.addKey(name: name, keyValue: keyValue, mtlsCertPath: mtlsCertPath) {
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
    func updateKey(id: String, name: String, keyValue: String, mtlsCertPath: String? = nil) -> Bool {
        let updated = KeychainStorage.shared.updateKey(id: id, name: name, keyValue: keyValue, mtlsCertPath: mtlsCertPath)
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

            // Send request with 4-byte big-endian length prefix
            if let requestData = try? JSONEncoder().encode(request) {
                let payloadLen = requestData.count
                var lenBuf: [UInt8] = [
                    UInt8((payloadLen >> 24) & 0xFF),
                    UInt8((payloadLen >> 16) & 0xFF),
                    UInt8((payloadLen >>  8) & 0xFF),
                    UInt8( payloadLen        & 0xFF),
                ]
                _ = write(fd, &lenBuf, 4)
                _ = requestData.withUnsafeBytes { ptr in
                    write(fd, ptr.baseAddress!, payloadLen)
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
        sendToHelper(HelperRequest(action: "ping", key: nil, fullTunnel: nil, binaryPath: nil, service: nil, mtlsCertPath: nil, excludeRoutes: nil, adaptiveMode: nil),                     timeoutSeconds: 2.0) { [weak self] response in
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

    func connect(key: String, fullTunnel: Bool = false, mtlsCertPath: String? = nil, excludeRoutes: String? = nil, adaptiveMode: Bool = false) {
        guard !isConnecting else { return }

        let normalizedKey = key.trimmingCharacters(in: CharacterSet.whitespacesAndNewlines)
            .replacingOccurrences(of: "aivpn://", with: "")

        savedKey = normalizedKey

        connectGeneration += 1
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
            service: nil,
            mtlsCertPath: mtlsCertPath,
            excludeRoutes: excludeRoutes,
            adaptiveMode: adaptiveMode
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

    // MARK: - Proxy Mode (no root required for ports > 1024)

    /// Launch aivpn-client directly as current user in SOCKS5 proxy mode.
    /// Does NOT go through the privileged helper — root is not needed for high ports.
    func connectProxy(key: String, proxyPort: Int) {
        guard !isConnecting else { return }

        let normalizedKey = key.trimmingCharacters(in: CharacterSet.whitespacesAndNewlines)
            .replacingOccurrences(of: "aivpn://", with: "")

        savedKey = normalizedKey

        // Warn if an mTLS cert is configured — it is silently ignored in proxy mode
        if let certPath = selectedKey?.mtlsCertPath, !certPath.isEmpty {
            print("Warning: mTLS certificate '\(certPath)' is not used in SOCKS5 proxy mode")
        }

        isConnecting = true
        isProxyMode = true
        lastError = nil
        bytesSent = 0
        bytesReceived = 0

        guard let binaryPath = helperClientBinaryPath(),
              FileManager.default.isExecutableFile(atPath: binaryPath) else {
            isConnecting = false
            isProxyMode = false
            lastError = "aivpn-client binary not found"
            return
        }

        // Clear log file
        FileManager.default.createFile(atPath: proxyLogPath, contents: nil)

        let process = Process()
        process.executableURL = URL(fileURLWithPath: binaryPath)
        process.arguments = ["-k", normalizedKey, "--proxy-listen", "127.0.0.1:\(proxyPort)"]
        var env = ProcessInfo.processInfo.environment
        env["RUST_LOG"] = "info"
        process.environment = env

        if let fh = FileHandle(forWritingAtPath: proxyLogPath) {
            process.standardOutput = fh
            process.standardError = fh
        }

        process.terminationHandler = { [weak self] _ in
            DispatchQueue.main.async {
                guard let self = self, self.isProxyMode else { return }
                self.stopProxyPoll()
                self.proxyProcess = nil
                self.isConnected = false
                self.isConnecting = false
                self.isProxyMode = false
            }
        }

        do {
            try process.run()
            proxyProcess = process
            startProxyPoll()
        } catch {
            isConnecting = false
            isProxyMode = false
            lastError = "Failed to start proxy: \(error.localizedDescription)"
        }
    }

    private func stopProxyMode() {
        proxyProcess?.terminate()
        proxyProcess = nil
        stopProxyPoll()
        DispatchQueue.main.async {
            self.isConnected = false
            self.isConnecting = false
            self.isProxyMode = false
        }
    }

    private func startProxyPoll() {
        proxyPollTimer?.invalidate()
        proxyPollTimer = Timer.scheduledTimer(withTimeInterval: 1.0, repeats: true) { [weak self] _ in
            self?.pollProxyLog()
        }
    }

    private func stopProxyPoll() {
        proxyPollTimer?.invalidate()
        proxyPollTimer = nil
    }

    private func pollProxyLog() {
        guard let log = try? String(contentsOfFile: proxyLogPath, encoding: .utf8) else { return }
        if log.contains("SOCKS5 proxy listening") {
            stopProxyPoll()
            DispatchQueue.main.async {
                self.isConnected = true
                self.isConnecting = false
            }
        } else if log.contains("ERROR") || log.contains("error") {
            let lines = log.components(separatedBy: "\n").filter { !$0.isEmpty }
            if let last = lines.last {
                DispatchQueue.main.async {
                    self.lastError = String(last.prefix(200))
                }
            }
        }
    }

    /// Run `aivpn-client bench --server <addr> --json` and return parsed result.
    /// Calls completion on the main thread; passes nil on failure.
    func runBench(serverAddr: String, completion: @escaping (BenchDisplayResult?) -> Void) {
        runBundledClientCommand(["bench", "--server", serverAddr, "--json"]) { success, output in
            guard success,
                  let data = output.data(using: .utf8),
                  let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                completion(nil)
                return
            }
            let result = BenchDisplayResult(
                p50: json["latency_p50_ms"] as? Double ?? 0,
                p95: json["latency_p95_ms"] as? Double ?? 0,
                p99: json["latency_p99_ms"] as? Double ?? 0,
                lossPct: json["packet_loss_pct"] as? Double ?? 0,
                qualityScore: json["quality_score"] as? Int ?? 0
            )
            completion(result)
        }
    }

    func disconnect() {
        if isProxyMode {
            stopProxyMode()
            return
        }

        let request = HelperRequest(action: "disconnect", key: nil, fullTunnel: nil, binaryPath: nil, service: nil, mtlsCertPath: nil, excludeRoutes: nil, adaptiveMode: nil)
        let disconnectGen = connectGeneration
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
                // Guard: skip state reset if connect() was called again before this
                // callback fired (stale disconnect clobbering new connection state).
                guard self.connectGeneration == disconnectGen else { return }
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
        sendToHelper(HelperRequest(action: "status", key: nil, fullTunnel: nil, binaryPath: nil, service: nil, mtlsCertPath: nil, excludeRoutes: nil, adaptiveMode: nil),                     timeoutSeconds: 2.0) { [weak self] response in
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
        sendToHelper(HelperRequest(action: "traffic", key: nil, fullTunnel: nil, binaryPath: nil, service: nil, mtlsCertPath: nil, excludeRoutes: nil, adaptiveMode: nil),                     timeoutSeconds: 1.0) { [weak self] response in
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

}
