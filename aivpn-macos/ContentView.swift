import SwiftUI

struct ContentView: View {
    @EnvironmentObject var vpn: VPNManager
    @EnvironmentObject var loc: LocalizationManager
    
    @State private var connectionKey: String = ""
    @State private var keyName: String = ""
    @State private var showKeyInput: Bool = false
    @State private var showConnectionKey: Bool = false
    @AppStorage("fullTunnel") private var fullTunnel: Bool = false
    @AppStorage("proxyMode") private var proxyMode: Bool = false
    @AppStorage("proxyPort") private var proxyPort: String = "1080"
    @AppStorage("adaptiveMode") private var adaptiveMode: Bool = false
    @State private var showDiagnostics: Bool = false
    @State private var benchRunning: Bool = false
    @State private var benchResult: BenchDisplayResult? = nil
    @State private var editingKeyId: String?
    @State private var editingKeyName: String = ""
    @State private var showDeleteConfirm = false
    @State private var keyToDelete: ConnectionKey?
    @State private var mtlsCertPath: String = ""
    @State private var recordingServiceName: String = ""
    private let recordingDarkGreen = Color(red: 0.0, green: 0.35, blue: 0.16)

    var body: some View {
        VStack(spacing: 0) {
            // Header
            HStack {
                Image(nsImage: NSApp.applicationIconImage)
                    .resizable()
                    .frame(width: 24, height: 24)
                Text("AIVPN")
                    .font(.headline)
                Spacer()
                // Language toggle
                Button(action: { loc.toggleLanguage() }) {
                    Text(loc.language == "en" ? "🇷🇺" : "🇬🇧")
                        .font(.title3)
                        .buttonStyle(.plain)
                }
                .buttonStyle(.plain)
                .help(loc.language == "en" ? "Русский" : "English")
            }
            .padding(.horizontal, 16)
            .padding(.top, 12)
            .padding(.bottom, 8)

            Divider()

            // Helper status indicator
            HStack {
                Circle()
                    .fill(vpn.helperAvailable ? Color.green : (vpn.isCheckingHelper ? .secondary : Color.orange))
                    .frame(width: 8, height: 8)
                Text(helperStatusText)
                    .font(.caption2)
                    .foregroundColor(helperStatusColor)
                Spacer()
            }
            .padding(.horizontal, 16)
            .padding(.top, 6)
            .padding(.bottom, 2)

            Divider()

            // Connection status
            VStack(spacing: 8) {
                HStack {
                    Circle()
                        .fill(vpn.isConnected ? Color.green : Color.gray)
                        .frame(width: 10, height: 10)
                    Text(vpn.isConnected ? loc.t("status_connected") :
                         vpn.isConnecting ? loc.t("connecting") :
                         loc.t("status_disconnected"))
                        .font(.subheadline)
                        .foregroundColor(vpn.isConnected ? .green : .secondary)
                    Spacer()
                }

                if vpn.isConnected {
                    HStack {
                        Text("↓ \(formatBytes(vpn.bytesReceived))")
                            .font(.caption)
                            .foregroundColor(.secondary)
                        Spacer()
                        Text("↑ \(formatBytes(vpn.bytesSent))")
                            .font(.caption)
                            .foregroundColor(.secondary)
                    }
                }

                if let error = vpn.lastError {
                    Text(error)
                        .font(.caption)
                        .foregroundColor(.red)
                        .lineLimit(2)
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 8)

            Divider()

            // Connection Keys List - ALWAYS VISIBLE
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    Text(loc.t("connection_keys"))
                        .font(.caption)
                        .fontWeight(.semibold)
                        .foregroundColor(.secondary)
                    Spacer()
                    Button(action: {
                        withAnimation {
                            showKeyInput = true
                            keyName = ""
                            connectionKey = ""
                        }
                    }) {
                        Image(systemName: "plus.circle.fill")
                            .font(.caption)
                    }
                    .buttonStyle(.plain)
                    .help(loc.t("add_key"))
                }
                
                if vpn.keys.isEmpty {
                    // Empty state
                    VStack(spacing: 8) {
                        Image(systemName: "key")
                            .font(.system(size: 32))
                            .foregroundColor(.secondary)
                        Text(loc.t("no_keys_yet"))
                            .font(.caption)
                            .foregroundColor(.secondary)
                        Button(loc.t("add_first_key")) {
                            withAnimation {
                                showKeyInput = true
                                keyName = ""
                                connectionKey = ""
                            }
                        }
                        .buttonStyle(.bordered)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 20)
                } else {
                    // Keys list
                    ScrollView {
                        LazyVStack(spacing: 6) {
                            ForEach(vpn.keys) { key in
                                KeyRowView(
                                    key: key,
                                    isSelected: vpn.selectedKeyId == key.id,
                                    isConnected: vpn.isConnected,
                                    onSelect: {
                                        vpn.selectKey(id: key.id)
                                    },
                                    onEdit: {
                                        editingKeyId = key.id
                                        editingKeyName = key.name
                                        keyName = key.name
                                        connectionKey = key.keyValue
                                        mtlsCertPath = key.mtlsCertPath ?? ""
                                        withAnimation {
                                            showKeyInput = true
                                        }
                                    },
                                    onDelete: {
                                        keyToDelete = key
                                        showDeleteConfirm = true
                                    }
                                )
                                .contentShape(Rectangle()) // Make entire row clickable
                            }
                        }
                        .padding(.vertical, 4)
                    }
                    .frame(maxHeight: 180)
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 8)
            .background(Color(NSColor.controlBackgroundColor).opacity(0.3))
            .cornerRadius(8)

            Divider()

            // Add Key Form (shown when adding new key)
            if showKeyInput {
                VStack(spacing: 8) {
                    TextField(loc.t("key_name"), text: $keyName)
                        .textFieldStyle(.roundedBorder)
                        .font(.system(size: 11))
                    
                    HStack(spacing: 4) {
                        if showConnectionKey {
                            TextField(loc.t("enter_key"), text: $connectionKey)
                                .textFieldStyle(.roundedBorder)
                                .font(.system(size: 10))
                                .help("aivpn://...")
                        } else {
                            SecureField(loc.t("enter_key"), text: $connectionKey)
                                .textFieldStyle(.roundedBorder)
                                .font(.system(size: 10))
                                .help("aivpn://...")
                        }
                        Button(action: { showConnectionKey.toggle() }) {
                            Image(systemName: showConnectionKey ? "eye.slash" : "eye")
                                .foregroundColor(.secondary)
                                .font(.system(size: 11))
                        }
                        .buttonStyle(.plain)
                    }

                    HStack {
                        Toggle(loc.t("full_tunnel"), isOn: $fullTunnel)
                            .toggleStyle(.checkbox)
                            .font(.caption)
                            .help(loc.t("full_tunnel_help"))
                            .disabled(proxyMode)
                        Spacer()
                    }

                    HStack {
                        Toggle(loc.t("proxy_mode"), isOn: $proxyMode)
                            .toggleStyle(.checkbox)
                            .font(.caption)
                            .help(loc.t("proxy_mode_help"))
                        Spacer()
                    }

                    if proxyMode {
                        HStack(spacing: 4) {
                            Text(loc.t("proxy_port"))
                                .font(.caption)
                                .foregroundColor(.secondary)
                            TextField("1080", text: $proxyPort)
                                .textFieldStyle(.roundedBorder)
                                .font(.system(size: 11))
                                .frame(width: 64)
                                .onReceive(proxyPort.publisher.collect()) { _ in
                                    let filtered = proxyPort.filter { $0.isNumber }
                                    if filtered != proxyPort { proxyPort = filtered }
                                }
                            Spacer()
                        }
                    }
                    
                    HStack {
                        Toggle(loc.t("adaptive_mode"), isOn: $adaptiveMode)
                            .toggleStyle(.checkbox)
                            .font(.caption)
                            .help(loc.t("adaptive_mode_help"))
                        Spacer()
                    }

                    TextField(loc.t("mtls_cert_path"), text: $mtlsCertPath)
                        .textFieldStyle(.roundedBorder)
                        .font(.system(size: 11))
                        .help(loc.t("mtls_cert_path_help"))

                    HStack(spacing: 8) {
                        Button(loc.t("cancel")) {
                            withAnimation {
                                showKeyInput = false
                                keyName = ""
                                connectionKey = ""
                                mtlsCertPath = ""
                                editingKeyId = nil
                            }
                        }
                        .buttonStyle(.bordered)
                        
                        Button(loc.t("save_key")) {
                            let name = keyName.isEmpty ? "Key \(vpn.keys.count + 1)" : keyName
                            
                            let cert = mtlsCertPath.trimmingCharacters(in: .whitespaces)
                            if let editId = editingKeyId {
                                if vpn.updateKey(id: editId, name: name, keyValue: connectionKey,
                                                 mtlsCertPath: cert.isEmpty ? nil : cert) {
                                    withAnimation {
                                        showKeyInput = false
                                        keyName = ""
                                        connectionKey = ""
                                        mtlsCertPath = ""
                                        editingKeyId = nil
                                    }
                                } else {
                                    vpn.lastError = loc.t("duplicate_key")
                                }
                            } else {
                                if vpn.addKey(name: name, keyValue: connectionKey,
                                              mtlsCertPath: cert.isEmpty ? nil : cert) {
                                    withAnimation {
                                        showKeyInput = false
                                        keyName = ""
                                        connectionKey = ""
                                        mtlsCertPath = ""
                                    }
                                } else {
                                    vpn.lastError = loc.t("duplicate_key")
                                }
                            }
                        }
                        .buttonStyle(.borderedProminent)
                        
                        Spacer()
                    }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 8)
                .transition(.opacity.combined(with: .move(edge: .top)))
            }

            Divider()

            if let result = vpn.lastRecordingResult {
                VStack(alignment: .leading, spacing: 8) {
                    HStack(alignment: .top) {
                        VStack(alignment: .leading, spacing: 4) {
                            Text(result.succeeded ? loc.t("recording_result_success_title") : loc.t("recording_result_failed_title"))
                                .font(.caption)
                                .fontWeight(.semibold)
                                .foregroundColor(result.succeeded ? recordingDarkGreen : .red)

                            Text(result.details)
                                .font(.caption)
                                .foregroundColor(.primary)
                                .fixedSize(horizontal: false, vertical: true)
                        }

                        Spacer()

                        Button(loc.t("dismiss")) {
                            vpn.clearRecordingResult()
                        }
                        .buttonStyle(.plain)
                        .font(.caption)
                    }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 8)

                Divider()
            }

            if vpn.isConnected && vpn.recordingCapabilityKnown && vpn.canRecordMasks {
                VStack(alignment: .leading, spacing: 8) {
                    HStack {
                        Text(loc.t("record_new_mask"))
                            .font(.caption)
                            .fontWeight(.semibold)
                            .foregroundColor(.secondary)
                        Spacer()
                    }

                    TextField(loc.t("record_service_name"), text: $recordingServiceName)
                        .textFieldStyle(.roundedBorder)
                        .font(.system(size: 11))

                    Button(action: {
                        switch vpn.recordingState {
                        case .recording, .starting:
                            vpn.stopMaskRecording()
                        default:
                            let trimmed = recordingServiceName.trimmingCharacters(in: .whitespacesAndNewlines)
                            let service = trimmed.isEmpty ? "mask_\(Int(Date().timeIntervalSince1970))" : trimmed
                            recordingServiceName = service
                            vpn.startMaskRecording(serviceName: service)
                        }
                    }) {
                        HStack {
                            Spacer()
                            Image(systemName: recordingButtonIcon)
                            Text(recordingButtonTitle)
                            Spacer()
                        }
                        .padding(.vertical, 6)
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(recordingButtonDisabled)

                    Text(recordingStatusText)
                        .font(.caption)
                        .foregroundColor(recordingStatusColor)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 8)

                Divider()
            }

            // Diagnostics panel (when connected)
            if vpn.isConnected {
                VStack(alignment: .leading, spacing: 8) {
                    HStack {
                        Text(loc.t("diagnostics"))
                            .font(.caption)
                            .fontWeight(.semibold)
                            .foregroundColor(.secondary)
                        Spacer()
                        Button(action: { showDiagnostics.toggle() }) {
                            Image(systemName: showDiagnostics ? "chevron.up" : "chevron.down")
                                .font(.caption2)
                        }
                        .buttonStyle(.plain)
                    }

                    if showDiagnostics {
                        if let result = benchResult {
                            VStack(alignment: .leading, spacing: 4) {
                                HStack {
                                    Text("Quality: \(result.qualityScore)/100")
                                        .font(.system(size: 12, weight: .semibold))
                                        .foregroundColor(result.qualityScore >= 80 ? .green :
                                                         result.qualityScore >= 50 ? .orange : .red)
                                    Spacer()
                                    Text("Loss: \(String(format: "%.1f", result.lossPct))%")
                                        .font(.caption)
                                        .foregroundColor(result.lossPct > 5 ? .red : .secondary)
                                }
                                Text("P50: \(Int(result.p50))ms  P95: \(Int(result.p95))ms  P99: \(Int(result.p99))ms")
                                    .font(.caption)
                                    .foregroundColor(.secondary)
                            }
                        } else if benchRunning {
                            HStack(spacing: 6) {
                                ProgressView().scaleEffect(0.6)
                                Text(loc.t("bench_running"))
                                    .font(.caption)
                                    .foregroundColor(.secondary)
                            }
                        } else {
                            Text(loc.t("bench_idle"))
                                .font(.caption)
                                .foregroundColor(.secondary)
                        }

                        Button(action: {
                            guard let key = vpn.selectedKey?.keyValue ?? vpn.keys.first?.keyValue,
                                  let addr = serverAddrFromConnectionKey(key) else { return }
                            benchRunning = true
                            benchResult = nil
                            vpn.runBench(serverAddr: addr) { result in
                                benchResult = result
                                benchRunning = false
                            }
                        }) {
                            HStack {
                                Spacer()
                                Text(benchRunning ? loc.t("bench_running") : loc.t("run_benchmark"))
                                Spacer()
                            }
                            .padding(.vertical, 4)
                        }
                        .buttonStyle(.bordered)
                        .disabled(benchRunning)
                        .font(.caption)
                    }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 8)
                .background(Color(NSColor.controlBackgroundColor).opacity(0.3))
                .cornerRadius(8)

                Divider()
            }

            // Connect / Disconnect button
            Button(action: {
                if vpn.isConnected {
                    vpn.disconnect()
                } else {
                    guard let selectedKey = vpn.selectedKey ?? vpn.keys.first else {
                        vpn.lastError = loc.t("no_key_selected")
                        return
                    }
                    
                    if !vpn.helperAvailable {
                        vpn.checkHelperAvailable()
                    } else {
                        if proxyMode, let port = Int(proxyPort), port > 1024 {
                            vpn.connectProxy(key: selectedKey.keyValue, proxyPort: port)
                        } else {
                            vpn.connect(key: selectedKey.keyValue, fullTunnel: fullTunnel,
                                        mtlsCertPath: selectedKey.mtlsCertPath)
                        }
                    }
                }
            }) {
                HStack {
                    Spacer()
                    if vpn.isConnecting {
                        ProgressView()
                            .scaleEffect(0.7)
                            .frame(width: 16, height: 16)
                        Text(loc.t("connecting"))
                    } else if vpn.isConnected {
                        Image(systemName: "stop.circle.fill")
                        Text(loc.t("disconnect"))
                    } else {
                        Image(systemName: "play.circle.fill")
                        Text(loc.t("connect"))
                    }
                    Spacer()
                }
                .padding(.vertical, 6)
                .foregroundStyle(connectButtonForegroundColor)
                .background(
                    RoundedRectangle(cornerRadius: 10)
                        .fill(connectButtonBackgroundColor)
                )
                .overlay(
                    RoundedRectangle(cornerRadius: 10)
                        .stroke(connectButtonBorderColor, lineWidth: connectButtonEnabled ? 0 : 1)
                )
            }
            .buttonStyle(.plain)
            .disabled(!connectButtonEnabled)
            .opacity(connectButtonEnabled ? 1.0 : 0.92)
            .padding(.horizontal, 16)
            .padding(.vertical, 8)

            Divider()

            // Footer
            HStack {
                Text("AIVPN v0.8.0")
                    .font(.caption2)
                    .foregroundColor(.secondary)
                Spacer()
                Button(loc.t("quit")) {
                    vpn.disconnect()
                    NSApp.terminate(nil)
                }
                .font(.caption2)
                .buttonStyle(.plain)
                .foregroundColor(.secondary)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 8)
        }
        .frame(width: 360)
        .frame(minHeight: 420)
        .onReceive(vpn.$isConnected) { connected in
            if let appDelegate = NSApp.delegate as? AppDelegate {
                appDelegate.updateStatusIcon(connected: connected)
            }
        }
        .confirmationDialog(loc.t("delete_key_confirm"), isPresented: $showDeleteConfirm) {
            Button(loc.t("delete"), role: .destructive) {
                if let key = keyToDelete {
                    vpn.deleteKey(id: key.id)
                }
                keyToDelete = nil
            }
            Button(loc.t("cancel"), role: .cancel) {}
        } message: {
            Text(loc.t("delete_key_message"))
        }
    }

    private func formatBytes(_ bytes: Int64) -> String {
        if bytes < 1024 { return "\(bytes) B" }
        if bytes < 1024 * 1024 { return String(format: "%.1f KB", Double(bytes) / 1024.0) }
        if bytes < 1024 * 1024 * 1024 { return String(format: "%.1f MB", Double(bytes) / 1024.0 / 1024.0) }
        return String(format: "%.1f GB", Double(bytes) / 1024.0 / 1024.0 / 1024.0)
    }

    private var helperStatusText: String {
        if vpn.helperAvailable {
            return loc.t("helper_ready")
        }
        if vpn.isCheckingHelper {
            return loc.t("helper_starting")
        }
        return loc.t("helper_missing")
    }

    private var helperStatusColor: Color {
        if vpn.helperAvailable || vpn.isCheckingHelper {
            return .secondary
        }
        return .orange
    }

    private var connectButtonEnabled: Bool {
        !vpn.isConnecting && vpn.helperAvailable && !vpn.keys.isEmpty
    }

    private var connectButtonBackgroundColor: Color {
        if vpn.isConnected {
            return .red
        }
        if vpn.isConnecting || connectButtonEnabled {
            return .blue
        }
        return Color(nsColor: .controlBackgroundColor)
    }

    private var connectButtonForegroundColor: Color {
        if vpn.isConnected || vpn.isConnecting || connectButtonEnabled {
            return .white
        }
        return Color(nsColor: .labelColor)
    }

    private var connectButtonBorderColor: Color {
        Color(nsColor: .separatorColor)
    }

    private var recordingStatusText: String {
        switch vpn.recordingState {
        case .idle:
            return loc.t("recording_ready")
        case .starting:
            return loc.t("recording_starting")
        case .recording:
            return loc.t("recording_active")
        case .stopping:
            return loc.t("recording_stopping")
        case .analyzing:
            return loc.t("recording_analyzing")
        case .success(_, let maskId):
            if let maskId, !maskId.isEmpty {
                return "\(loc.t("recording_success")): \(maskId)"
            }
            return loc.t("recording_success")
        case .failed(_, let reason):
            let lowerReason = reason.lowercased()
            if lowerReason.contains("self-test") || lowerReason.contains("verification") || lowerReason.contains("провер") {
                return loc.t("recording_self_test_failed") + ": " + reason
            }
            return loc.t("recording_failed") + ": " + reason
        }
    }

    private var recordingStatusColor: Color {
        switch vpn.recordingState {
        case .idle:
            return recordingDarkGreen
        case .starting, .recording, .stopping, .analyzing:
            return recordingDarkGreen
        case .success:
            return recordingDarkGreen
        case .failed:
            return .red
        }
    }

    private var recordingButtonTitle: String {
        switch vpn.recordingState {
        case .recording, .starting:
            return loc.t("stop_recording")
        default:
            return loc.t("record_new_mask")
        }
    }

    private var recordingButtonIcon: String {
        switch vpn.recordingState {
        case .recording, .starting:
            return "stop.circle.fill"
        default:
            return "waveform.badge.magnifyingglass"
        }
    }

    private var recordingButtonDisabled: Bool {
        switch vpn.recordingState {
        case .stopping, .analyzing:
            return true
        default:
            return false
        }
    }
}

// MARK: - Bench Display

/// Extract server address from an `aivpn://` connection key (base64url JSON ["s"] field).
func serverAddrFromConnectionKey(_ key: String) -> String? {
    guard key.hasPrefix("aivpn://") else { return nil }
    var b64 = String(key.dropFirst(8))
        .replacingOccurrences(of: "-", with: "+")
        .replacingOccurrences(of: "_", with: "/")
    while b64.count % 4 != 0 { b64 += "=" }
    guard let data = Data(base64Encoded: b64),
          let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
          let s = json["s"] as? String else { return nil }
    return s
}

struct BenchDisplayResult {
    let p50: Double
    let p95: Double
    let p99: Double
    let lossPct: Double
    let qualityScore: Int
}

// MARK: - Key Row View

struct KeyRowView: View {
    let key: ConnectionKey
    let isSelected: Bool
    let isConnected: Bool
    let onSelect: () -> Void
    let onEdit: () -> Void
    let onDelete: () -> Void

    @EnvironmentObject var loc: LocalizationManager
    @State private var isHovering = false
    
    var body: some View {
        HStack(spacing: 10) {
            // Selection radio button - larger and more visible
            Circle()
                .fill(isSelected ? Color.green : Color.clear)
                .overlay(
                    Circle()
                        .stroke(isSelected ? Color.green : Color.gray, lineWidth: 2)
                )
                .frame(width: 16, height: 16)
            
            // Key info - full width clickable
            VStack(alignment: .leading, spacing: 3) {
                Text(key.name)
                    .font(.system(size: 12))
                    .fontWeight(isSelected ? .semibold : .regular)

                if key.isRecordingAdminKey {
                    Text("recording-admin")
                        .font(.system(size: 9, weight: .medium))
                        .foregroundColor(.orange)
                }
                
                if let server = key.serverAddress {
                    HStack(spacing: 4) {
                        Image(systemName: "server.rack")
                            .font(.system(size: 9))
                        Text(server)
                            .font(.system(size: 10))
                            .foregroundColor(.secondary)
                    }
                }
                if let vpnIP = key.vpnIP {
                    HStack(spacing: 4) {
                        Image(systemName: "network")
                            .font(.system(size: 9))
                        Text(vpnIP)
                            .font(.system(size: 10))
                            .foregroundColor(.secondary)
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            // Connection status indicator
            if isSelected && isConnected {
                Image(systemName: "checkmark.circle.fill")
                    .foregroundColor(.green)
                    .font(.system(size: 16))
                    .padding(.trailing, 4)
            }
            
            // Actions menu - larger button
            Menu {
                Button(action: onEdit) {
                    Label(loc.t("edit"), systemImage: "pencil")
                }
                Button(role: .destructive, action: onDelete) {
                    Label(loc.t("delete"), systemImage: "trash")
                }
            } label: {
                Image(systemName: "ellipsis.circle")
                    .foregroundColor(.secondary)
                    .font(.system(size: 14))
                    .padding(4)
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .background(
            RoundedRectangle(cornerRadius: 8)
                .fill(isSelected ? Color.green.opacity(0.1) : 
                      isHovering ? Color.gray.opacity(0.05) : Color.clear)
        )
        .overlay(
            RoundedRectangle(cornerRadius: 8)
                .stroke(isSelected ? Color.green.opacity(0.4) : Color.clear, lineWidth: 1.5)
        )
        .onHover { hovering in
            isHovering = hovering
        }
        .onTapGesture {
            onSelect()
        }
        // Make entire row clickable
        .contentShape(Rectangle())
    }
}
