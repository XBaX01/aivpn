import Foundation
import NetworkExtension
import UserNotifications
import Combine

// MARK: - Recording state (mirrors macOS VPNManager)

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
}

// MARK: - IPC message types (main app <-> tunnel extension)

private enum TunnelMessageType: String {
    case getTraffic   = "get_traffic"
    case startRecord  = "record_start"
    case stopRecord   = "record_stop"
    case getRecordStatus = "record_status"
}

// MARK: - VPNManager

class VPNManager: ObservableObject {
    static let shared = VPNManager()

    @Published var isConnected: Bool = false
    @Published var isConnecting: Bool = false
    @Published var lastError: String?
    @Published var bytesSent: Int64 = 0
    @Published var bytesReceived: Int64 = 0
    @Published var connectionDuration: TimeInterval = 0
    @Published var recordingState: MaskRecordingState = .idle
    @Published var canRecordMasks: Bool = false
    @Published var recordingCapabilityKnown: Bool = false
    @Published var lastRecordingResult: RecordingResultSummary?

    var keys: [ConnectionKey] { KeychainStorage.shared.keys }
    var selectedKeyId: String? {
        get { KeychainStorage.shared.selectedKeyId }
        set { KeychainStorage.shared.selectKey(id: newValue) }
    }
    var selectedKey: ConnectionKey? { KeychainStorage.shared.activeKey }

    private var manager: NETunnelProviderManager?
    private var statusObserver: NSObjectProtocol?
    private var trafficTimer: Timer?
    private var durationTimer: Timer?
    private var connectionStartDate: Date?
    private let bundleId = "com.aivpn.client.tunnel"

    init() {
        KeychainStorage.shared.loadKeys()
        loadManager()
        requestNotificationPermission()
    }

    // MARK: - Manager lifecycle

    private func loadManager() {
        NETunnelProviderManager.loadAllFromPreferences { [weak self] managers, error in
            guard let self = self else { return }
            if let m = managers?.first(where: {
                ($0.protocolConfiguration as? NETunnelProviderProtocol)?.providerBundleIdentifier == self.bundleId
            }) {
                self.manager = m
            } else {
                let m = NETunnelProviderManager()
                let proto = NETunnelProviderProtocol()
                proto.providerBundleIdentifier = self.bundleId
                proto.serverAddress = "aivpn"
                m.protocolConfiguration = proto
                m.localizedDescription = "AIVPN"
                self.manager = m
            }
            self.observeStatus()
            // loadAllFromPreferences calls back on a private queue; marshal to main
            // before touching @Published properties via syncStatus().
            DispatchQueue.main.async { self.syncStatus() }
        }
    }

    private func observeStatus() {
        if let obs = statusObserver {
            NotificationCenter.default.removeObserver(obs)
        }
        statusObserver = NotificationCenter.default.addObserver(
            forName: .NEVPNStatusDidChange, object: manager?.connection, queue: .main
        ) { [weak self] _ in
            self?.syncStatus()
        }
    }

    private func syncStatus() {
        guard let connection = manager?.connection else { return }
        let status = connection.status
        switch status {
        case .connected:
            isConnecting = false
            isConnected = true
            if connectionStartDate == nil {
                connectionStartDate = Date()
                startTimers()
            }
        case .connecting, .reasserting:
            isConnecting = true
            isConnected = false
        case .disconnecting:
            isConnecting = false
        case .disconnected, .invalid:
            isConnecting = false
            if isConnected {
                isConnected = false
                stopTimers()
                connectionStartDate = nil
                bytesSent = 0
                bytesReceived = 0
                connectionDuration = 0
                recordingState = .idle
                canRecordMasks = false
                recordingCapabilityKnown = false
            }
        @unknown default:
            break
        }
    }

    // MARK: - Connect / Disconnect

    func connect(key: ConnectionKey, fullTunnel: Bool, adaptiveLevel: Int = 0) {
        guard let manager = manager else { return }
        guard !isConnecting else { return }

        isConnecting = true
        lastError = nil
        bytesSent = 0
        bytesReceived = 0
        connectionDuration = 0
        recordingState = .idle
        canRecordMasks = false
        recordingCapabilityKnown = false
        lastRecordingResult = nil

        let proto = NETunnelProviderProtocol()
        proto.providerBundleIdentifier = bundleId
        proto.serverAddress = key.serverAddress ?? "aivpn"
        var providerConfig: [String: Any] = [
            "key": key.fullKey,
            "fullTunnel": fullTunnel,
        ]
        if adaptiveLevel > 0 {
            providerConfig["adaptiveLevel"] = adaptiveLevel
        }
        if let cert = key.mtlsCert, !cert.isEmpty {
            providerConfig["mtlsCert"] = cert
        }

        // Pass split-tunnel lists from App Group UserDefaults to the tunnel extension.
        // The tunnel reads these from providerConfiguration because the extension
        // process cannot share in-memory state with the app process.
        let splitDefaults = UserDefaults(suiteName: "group.com.aivpn.client")
        if let routes = splitDefaults?.stringArray(forKey: "excluded_routes"), !routes.isEmpty {
            providerConfig["excluded_routes"] = routes.joined(separator: ",")
        }
        if let domains = splitDefaults?.stringArray(forKey: "excluded_domains"), !domains.isEmpty {
            providerConfig["excluded_domains"] = domains.joined(separator: ",")
        }

        proto.providerConfiguration = providerConfig
        manager.protocolConfiguration = proto
        manager.localizedDescription = "AIVPN"
        manager.isEnabled = true

        manager.saveToPreferences { [weak self] error in
            guard let self = self else { return }
            if let error = error {
                DispatchQueue.main.async {
                    self.isConnecting = false
                    self.lastError = error.localizedDescription
                }
                return
            }
            self.observeStatus()
            do {
                try (self.manager?.connection as? NETunnelProviderSession)?.startTunnel(options: nil)
            } catch {
                DispatchQueue.main.async {
                    self.isConnecting = false
                    self.lastError = error.localizedDescription
                }
            }
        }
    }

    func disconnect() {
        (manager?.connection as? NETunnelProviderSession)?.stopTunnel()
    }

    // MARK: - Key management (delegates to KeychainStorage)

    func addKey(name: String, keyValue: String, mtlsCert: String? = nil) -> Bool {
        guard let k = KeychainStorage.shared.addKey(name: name, keyValue: keyValue, mtlsCert: mtlsCert) else { return false }
        KeychainStorage.shared.selectKey(id: k.id)
        objectWillChange.send()
        return true
    }

    func deleteKey(id: String) {
        KeychainStorage.shared.deleteKey(id: id)
        objectWillChange.send()
    }

    func updateKey(id: String, name: String, keyValue: String, mtlsCert: String? = nil) -> Bool {
        let ok = KeychainStorage.shared.updateKey(id: id, name: name, keyValue: keyValue, mtlsCert: mtlsCert)
        if ok { objectWillChange.send() }
        return ok
    }

    func selectKey(id: String) {
        KeychainStorage.shared.selectKey(id: id)
        objectWillChange.send()
    }

    // MARK: - Traffic stats (IPC to tunnel extension)

    private func startTimers() {
        trafficTimer = Timer.scheduledTimer(withTimeInterval: 1.0, repeats: true) { [weak self] _ in
            self?.fetchTrafficStats()
        }
        durationTimer = Timer.scheduledTimer(withTimeInterval: 1.0, repeats: true) { [weak self] _ in
            guard let self = self, let start = self.connectionStartDate else { return }
            self.connectionDuration = Date().timeIntervalSince(start)
        }
    }

    private func stopTimers() {
        trafficTimer?.invalidate(); trafficTimer = nil
        durationTimer?.invalidate(); durationTimer = nil
    }

    private func fetchTrafficStats() {
        sendMessage(type: .getTraffic, body: [:]) { [weak self] response in
            guard let self = self, let r = response else { return }
            if let up = r["upload"] as? Int64 { self.bytesSent = up }
            if let down = r["download"] as? Int64 { self.bytesReceived = down }
            if let canRec = r["can_record"] as? Bool {
                self.canRecordMasks = canRec
                self.recordingCapabilityKnown = true
            }
            if let stateStr = r["recording_state"] as? String {
                self.applyRecordingState(stateStr,
                    service: r["service"] as? String ?? "",
                    maskId: r["mask_id"] as? String,
                    message: r["message"] as? String)
            }
        }
    }

    // MARK: - Mask recording

    func startMaskRecording(serviceName: String) {
        guard isConnected, canRecordMasks else { return }
        let name = serviceName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty else { return }
        recordingState = .starting(service: name)
        lastRecordingResult = nil
        sendMessage(type: .startRecord, body: ["service": name], completion: nil)
    }

    func stopMaskRecording() {
        guard case .recording(let svc) = recordingState else { return }
        recordingState = .stopping(service: svc)
        sendMessage(type: .stopRecord, body: [:], completion: nil)
    }

    func clearRecordingResult() { lastRecordingResult = nil }

    private func applyRecordingState(_ state: String, service: String, maskId: String?, message: String?) {
        switch state {
        case "recording":  recordingState = .recording(service: service)
        case "stopping":   recordingState = .stopping(service: service)
        case "analyzing":  recordingState = .analyzing(service: service)
        case "success":
            recordingState = .success(service: service, maskId: maskId)
            let details = maskId.map { "Mask saved. ID: \($0)" } ?? "Mask saved successfully."
            lastRecordingResult = RecordingResultSummary(succeeded: true, title: "Mask recorded", details: details)
            postNotification(title: "Mask recorded", body: details)
        case "failed":
            let reason = message ?? "Recording failed"
            recordingState = .failed(service: service, reason: reason)
            lastRecordingResult = RecordingResultSummary(succeeded: false, title: "Recording failed", details: reason)
            postNotification(title: "Mask recording failed", body: reason)
        default:
            recordingState = .idle
        }
    }

    // MARK: - IPC helper

    private func sendMessage(type: TunnelMessageType, body: [String: Any],
                             completion: (([String: Any]?) -> Void)?) {
        guard let session = manager?.connection as? NETunnelProviderSession,
              manager?.connection.status == .connected else {
            completion?(nil)
            return
        }
        var payload = body
        payload["type"] = type.rawValue
        guard let data = try? JSONSerialization.data(withJSONObject: payload) else {
            completion?(nil)
            return
        }
        do {
            try session.sendProviderMessage(data) { responseData in
                guard let rd = responseData,
                      let json = try? JSONSerialization.jsonObject(with: rd) as? [String: Any]
                else {
                    DispatchQueue.main.async { completion?(nil) }
                    return
                }
                DispatchQueue.main.async { completion?(json) }
            }
        } catch {
            completion?(nil)
        }
    }

    // MARK: - Notifications

    private func requestNotificationPermission() {
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound]) { _, _ in }
    }

    private func postNotification(title: String, body: String) {
        let c = UNMutableNotificationContent()
        c.title = title; c.body = body; c.sound = .default
        UNUserNotificationCenter.current().add(
            UNNotificationRequest(identifier: UUID().uuidString, content: c, trigger: nil))
    }

    deinit {
        stopTimers()
        if let obs = statusObserver { NotificationCenter.default.removeObserver(obs) }
    }
}
