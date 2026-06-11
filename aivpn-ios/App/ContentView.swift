import SwiftUI
import NetworkExtension

// MARK: - Helpers

private func formatBytes(_ bytes: Int64) -> String {
    let kb = Double(bytes) / 1024
    let mb = kb / 1024
    let gb = mb / 1024
    if gb >= 1 { return String(format: "%.2f GB", gb) }
    if mb >= 1 { return String(format: "%.2f MB", mb) }
    if kb >= 1 { return String(format: "%.1f KB", kb) }
    return "\(bytes) B"
}

private func formatDuration(_ t: TimeInterval) -> String {
    let h = Int(t) / 3600
    let m = (Int(t) % 3600) / 60
    let s = Int(t) % 60
    if h > 0 { return String(format: "%02d:%02d:%02d", h, m, s) }
    return String(format: "%02d:%02d", m, s)
}

// MARK: - Key row

private struct KeyRowView: View {
    let key: ConnectionKey
    let isSelected: Bool
    let onSelect: () -> Void
    let onEdit: () -> Void
    let onDelete: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: isSelected ? "checkmark.circle.fill" : "circle")
                .foregroundColor(isSelected ? .accentColor : .secondary)
                .font(.title3)
                .onTapGesture { onSelect() }

            VStack(alignment: .leading, spacing: 2) {
                Text(key.name)
                    .font(.body)
                    .fontWeight(isSelected ? .semibold : .regular)
                if let s = key.serverAddress {
                    Text(s)
                        .font(.caption)
                        .foregroundColor(.secondary)
                }
            }
            Spacer()
            if key.isRecordingAdminKey {
                Image(systemName: "antenna.radiowaves.left.and.right")
                    .foregroundColor(.orange)
                    .font(.caption)
            }
            Menu {
                Button { onEdit() }  label: { Label("Edit",   systemImage: "pencil") }
                Button(role: .destructive) { onDelete() } label: { Label("Delete", systemImage: "trash") }
            } label: {
                Image(systemName: "ellipsis.circle")
                    .foregroundColor(.secondary)
            }
        }
        .padding(.vertical, 4)
        .contentShape(Rectangle())
        .onTapGesture { onSelect() }
    }
}

// MARK: - Add / Edit key sheet

private struct KeyEditSheet: View {
    let existingKey: ConnectionKey?
    let onSave: (String, String) -> Bool
    let onCancel: () -> Void

    @State private var name: String
    @State private var value: String
    @State private var error: String?
    @EnvironmentObject private var loc: LocalizationManager

    init(existingKey: ConnectionKey?, onSave: @escaping (String, String) -> Bool, onCancel: @escaping () -> Void) {
        self.existingKey = existingKey
        self.onSave = onSave
        self.onCancel = onCancel
        _name  = State(initialValue: existingKey?.name ?? "")
        _value = State(initialValue: existingKey.map { "aivpn://\($0.keyValue)" } ?? "")
    }

    var body: some View {
        NavigationView {
            Form {
                Section {
                    TextField(loc.t("key_name"), text: $name)
                        .autocorrectionDisabled()
                    TextField(loc.t("enter_key"), text: $value, axis: .vertical)
                        .autocorrectionDisabled()
                        .autocapitalization(.none)
                        .lineLimit(3...6)
                }
                if let e = error {
                    Section { Text(e).foregroundColor(.red).font(.caption) }
                }
            }
            .navigationTitle(existingKey == nil ? loc.t("add_key") : loc.t("edit"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(loc.t("cancel")) { onCancel() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(loc.t("save_key")) {
                        let ok = onSave(name.trimmingCharacters(in: .whitespaces),
                                        value.trimmingCharacters(in: .whitespaces))
                        if !ok { error = loc.t("duplicate_key") }
                    }
                    .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty ||
                              value.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
        }
    }
}

// MARK: - Status ring

private struct StatusRing: View {
    let isConnected: Bool
    let isConnecting: Bool

    private var color: Color {
        isConnected ? .green : (isConnecting ? .orange : .red)
    }
    private var symbol: String {
        isConnected ? "lock.fill" : (isConnecting ? "arrow.triangle.2.circlepath" : "lock.open.fill")
    }

    var body: some View {
        ZStack {
            Circle()
                .stroke(color.opacity(0.2), lineWidth: 10)
                .frame(width: 120, height: 120)
            Circle()
                .trim(from: 0, to: isConnecting ? 0.6 : 1)
                .stroke(color, style: StrokeStyle(lineWidth: 10, lineCap: .round))
                .frame(width: 120, height: 120)
                .rotationEffect(.degrees(-90))
                .animation(isConnecting
                    ? .linear(duration: 1.2).repeatForever(autoreverses: false)
                    : .easeInOut, value: isConnecting)
            Image(systemName: symbol)
                .font(.system(size: 36, weight: .medium))
                .foregroundColor(color)
        }
    }
}

// MARK: - Recording section

private struct RecordingSection: View {
    @EnvironmentObject var vpn: VPNManager
    @EnvironmentObject var loc: LocalizationManager
    @State private var serviceName: String = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(loc.t("record_new_mask"))
                .font(.headline)

            switch vpn.recordingState {
            case .idle:
                HStack {
                    TextField(loc.t("record_service_name"), text: $serviceName)
                        .textFieldStyle(.roundedBorder)
                        .autocorrectionDisabled()
                    Button(loc.t("record_new_mask")) {
                        vpn.startMaskRecording(serviceName: serviceName)
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(serviceName.trimmingCharacters(in: .whitespaces).isEmpty)
                }

            case .starting(let svc):
                Label(loc.t("recording_starting"), systemImage: "record.circle")
                    .foregroundColor(.orange)
                Text(svc).font(.caption).foregroundColor(.secondary)

            case .recording(let svc):
                VStack(alignment: .leading, spacing: 6) {
                    Label(loc.t("recording_active"), systemImage: "record.circle.fill")
                        .foregroundColor(.red)
                    Text(svc).font(.caption).foregroundColor(.secondary)
                    Button(loc.t("stop_recording"), role: .destructive) {
                        vpn.stopMaskRecording()
                    }
                    .buttonStyle(.bordered)
                }

            case .stopping:
                Label(loc.t("recording_stopping"), systemImage: "stop.circle")
                    .foregroundColor(.orange)

            case .analyzing:
                Label(loc.t("recording_analyzing"), systemImage: "waveform.path.ecg")
                    .foregroundColor(.blue)

            case .success(let svc, _):
                Label(loc.t("recording_success"), systemImage: "checkmark.seal.fill")
                    .foregroundColor(.green)
                Text(svc).font(.caption).foregroundColor(.secondary)
                Button(loc.t("dismiss")) {
                    vpn.recordingState = .idle
                    serviceName = ""
                }
                .font(.caption)

            case .failed(let svc, let reason):
                VStack(alignment: .leading, spacing: 4) {
                    Label(loc.t("recording_failed"), systemImage: "xmark.octagon.fill")
                        .foregroundColor(.red)
                    Text("\(svc): \(reason)").font(.caption).foregroundColor(.secondary)
                    Button(loc.t("dismiss")) {
                        vpn.recordingState = .idle
                    }
                    .font(.caption)
                }
            }

            if let result = vpn.lastRecordingResult {
                Divider()
                HStack {
                    Image(systemName: result.succeeded ? "checkmark.circle.fill" : "xmark.circle.fill")
                        .foregroundColor(result.succeeded ? .green : .red)
                    VStack(alignment: .leading) {
                        Text(result.title).font(.caption).bold()
                        Text(result.details).font(.caption2).foregroundColor(.secondary)
                    }
                    Spacer()
                    Button(loc.t("dismiss")) { vpn.clearRecordingResult() }
                        .font(.caption2)
                }
            }
        }
        .padding()
        .background(Color(.secondarySystemBackground))
        .cornerRadius(12)
    }
}

// MARK: - Main ContentView

struct ContentView: View {
    @EnvironmentObject var vpn: VPNManager
    @EnvironmentObject var loc: LocalizationManager

    @State private var fullTunnel: Bool = true
    @State private var showAddKey: Bool = false
    @State private var editingKey: ConnectionKey?
    @State private var deleteKeyId: String?
    @State private var showDeleteConfirm: Bool = false
    @State private var showSplitTunnel: Bool = false

    var body: some View {
        NavigationView {
            ScrollView {
                VStack(spacing: 20) {
                    statusSection
                    trafficSection
                    keyListSection
                    connectSection
                    if vpn.isConnected && vpn.recordingCapabilityKnown && vpn.canRecordMasks {
                        RecordingSection()
                            .padding(.horizontal)
                            .environmentObject(vpn)
                            .environmentObject(loc)
                    }
                    footerSection
                }
                .padding(.vertical)
            }
            .navigationTitle("AIVPN")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { toolbarContent }
        }
        .navigationViewStyle(.stack)
        .sheet(isPresented: $showAddKey) {
            KeyEditSheet(existingKey: nil,
                onSave: { name, val in
                    let ok = vpn.addKey(name: name, keyValue: val)
                    if ok { showAddKey = false }
                    return ok
                },
                onCancel: { showAddKey = false }
            )
            .environmentObject(loc)
        }
        .sheet(item: $editingKey) { key in
            KeyEditSheet(existingKey: key,
                onSave: { name, val in
                    let ok = vpn.updateKey(id: key.id, name: name, keyValue: val)
                    if ok { editingKey = nil }
                    return ok
                },
                onCancel: { editingKey = nil }
            )
            .environmentObject(loc)
        }
        .confirmationDialog(loc.t("delete_key_confirm"),
                            isPresented: $showDeleteConfirm,
                            titleVisibility: .visible) {
            Button(loc.t("delete"), role: .destructive) {
                if let id = deleteKeyId { vpn.deleteKey(id: id) }
                deleteKeyId = nil
            }
            Button(loc.t("cancel"), role: .cancel) { deleteKeyId = nil }
        } message: {
            Text(loc.t("delete_key_message"))
        }
        .sheet(isPresented: $showSplitTunnel) {
            SplitTunnelView()
                .environmentObject(loc)
        }
    }

    // MARK: - Status section

    private var statusSection: some View {
        VStack(spacing: 10) {
            StatusRing(isConnected: vpn.isConnected, isConnecting: vpn.isConnecting)
            Text(statusLabel)
                .font(.headline)
                .foregroundColor(statusColor)
            if let err = vpn.lastError {
                Text(err)
                    .font(.caption)
                    .foregroundColor(.red)
                    .multilineTextAlignment(.center)
                    .padding(.horizontal)
            }
        }
    }

    private var statusLabel: String {
        if vpn.isConnecting { return loc.t("status_connecting") }
        return vpn.isConnected ? loc.t("status_connected") : loc.t("status_disconnected")
    }

    private var statusColor: Color {
        vpn.isConnected ? .green : (vpn.isConnecting ? .orange : .secondary)
    }

    // MARK: - Traffic stats

    @ViewBuilder
    private var trafficSection: some View {
        if vpn.isConnected {
            HStack(spacing: 0) {
                statCell(icon: "arrow.up.circle.fill", color: .blue,
                         label: loc.t("upload"), value: formatBytes(vpn.bytesSent))
                Divider().frame(height: 44)
                statCell(icon: "arrow.down.circle.fill", color: .green,
                         label: loc.t("download"), value: formatBytes(vpn.bytesReceived))
                Divider().frame(height: 44)
                statCell(icon: "clock.fill", color: .orange,
                         label: loc.t("duration"), value: formatDuration(vpn.connectionDuration))
            }
            .padding(.horizontal)
            .background(Color(.secondarySystemBackground))
            .cornerRadius(12)
            .padding(.horizontal)
        }
    }

    private func statCell(icon: String, color: Color, label: String, value: String) -> some View {
        VStack(spacing: 4) {
            Image(systemName: icon).foregroundColor(color).font(.title3)
            Text(value).font(.system(.body, design: .monospaced)).bold()
            Text(label).font(.caption2).foregroundColor(.secondary)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 8)
    }

    // MARK: - Keys list

    private var keyListSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(loc.t("connection_keys"))
                    .font(.headline)
                Spacer()
                Button { showAddKey = true } label: {
                    Image(systemName: "plus.circle.fill")
                        .font(.title3)
                }
            }
            .padding(.horizontal)

            if vpn.keys.isEmpty {
                VStack(spacing: 8) {
                    Text(loc.t("no_keys_yet")).foregroundColor(.secondary)
                    Button(loc.t("add_first_key")) { showAddKey = true }
                        .buttonStyle(.bordered)
                }
                .frame(maxWidth: .infinity)
                .padding()
            } else {
                VStack(spacing: 0) {
                    ForEach(vpn.keys) { key in
                        KeyRowView(
                            key: key,
                            isSelected: vpn.selectedKeyId == key.id,
                            onSelect: { vpn.selectKey(id: key.id) },
                            onEdit: { editingKey = key },
                            onDelete: {
                                deleteKeyId = key.id
                                showDeleteConfirm = true
                            }
                        )
                        .padding(.horizontal)
                        if key.id != vpn.keys.last?.id { Divider().padding(.leading) }
                    }
                }
                .background(Color(.secondarySystemBackground))
                .cornerRadius(12)
                .padding(.horizontal)
            }
        }
    }

    // MARK: - Connect / disconnect

    private var connectSection: some View {
        VStack(spacing: 10) {
            if !vpn.isConnected {
                Toggle(loc.t("full_tunnel"), isOn: $fullTunnel)
                    .padding(.horizontal)
            }
            HStack(spacing: 12) {
                Button {
                    if vpn.isConnected {
                        vpn.disconnect()
                    } else {
                        guard let key = vpn.selectedKey else { return }
                        vpn.connect(key: key, fullTunnel: fullTunnel)
                    }
                } label: {
                    Label(
                        vpn.isConnected ? loc.t("disconnect") : loc.t("connect"),
                        systemImage: vpn.isConnected ? "stop.circle.fill" : "play.circle.fill"
                    )
                    .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .tint(vpn.isConnected ? .red : .accentColor)
                .disabled(vpn.isConnecting || (!vpn.isConnected && vpn.selectedKey == nil))
                .padding(.horizontal)

                if vpn.isConnected {
                    Button {
                        showSplitTunnel = true
                    } label: {
                        Image(systemName: "network")
                    }
                    .buttonStyle(.bordered)
                    .padding(.trailing)
                }
            }
        }
    }

    // MARK: - Footer

    private var footerSection: some View {
        Text(loc.t("version_footer"))
            .font(.caption2)
            .foregroundColor(.secondary)
            .padding(.top, 8)
    }

    // MARK: - Toolbar

    @ToolbarContentBuilder
    private var toolbarContent: some ToolbarContent {
        ToolbarItem(placement: .navigationBarLeading) {
            Button {
                loc.toggleLanguage()
            } label: {
                Text(loc.language == "en" ? "RU" : "EN")
                    .font(.caption)
                    .padding(4)
                    .background(Color(.secondarySystemBackground))
                    .cornerRadius(6)
            }
        }
        ToolbarItem(placement: .navigationBarTrailing) {
            Button {
                showSplitTunnel = true
            } label: {
                Image(systemName: "network")
            }
        }
    }
}
