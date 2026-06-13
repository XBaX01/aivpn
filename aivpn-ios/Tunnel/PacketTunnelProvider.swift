import NetworkExtension
import Darwin

// PacketTunnelProvider bridges NEPacketTunnelProvider to the Rust aivpn-ios-core
// static library via a socketpair.
//
// Data path:
//   packetFlow ↔ sp[0]  (Swift bridge loops)
//              sp[1]  (Rust dup()s on entry, owns its copy)
//
// IPC from the main app arrives via handleAppMessage(_:completionHandler:).

class PacketTunnelProvider: NEPacketTunnelProvider {

    private var sp: [Int32] = [-1, -1]
    private var rustThread: Thread?
    private var outboundTask: Task<Void, Never>?
    private var isStopped = false
    private let appGroup = "group.com.aivpn.client"
    // Recording requires full Rust control-plane wiring — not yet implemented
    private let canRecord: Bool = false

    // MARK: - Start

    override func startTunnel(options: [String: NSObject]?,
                              completionHandler: @escaping (Error?) -> Void) {
        guard let proto = protocolConfiguration as? NETunnelProviderProtocol,
              let cfg   = proto.providerConfiguration,
              let keyStr = cfg["key"] as? String else {
            completionHandler(makeError("missing provider configuration"))
            return
        }

        let fullTunnel = cfg["fullTunnel"] as? Bool ?? true

        guard let key = TunnelConnectionKey(rawKey: keyStr) else {
            completionHandler(makeError("invalid connection key"))
            return
        }

        // Create socketpair: sp[0] = Swift side, sp[1] = Rust side
        var fds: [Int32] = [-1, -1]
        guard socketpair(AF_UNIX, SOCK_DGRAM, 0, &fds) == 0 else {
            completionHandler(makeError("socketpair failed: \(errno)"))
            return
        }
        sp = fds
        isStopped = false

        var bufSize: Int32 = 65536
        setsockopt(sp[0], SOL_SOCKET, SO_SNDBUF, &bufSize, socklen_t(MemoryLayout<Int32>.size))
        setsockopt(sp[0], SOL_SOCKET, SO_RCVBUF, &bufSize, socklen_t(MemoryLayout<Int32>.size))
        setsockopt(sp[1], SOL_SOCKET, SO_SNDBUF, &bufSize, socklen_t(MemoryLayout<Int32>.size))
        setsockopt(sp[1], SOL_SOCKET, SO_RCVBUF, &bufSize, socklen_t(MemoryLayout<Int32>.size))

        // sp[0] must be non-blocking so Darwin.read in the outbound loop returns
        // EAGAIN instead of blocking the cooperative thread-pool thread.
        let flags = fcntl(sp[0], F_GETFL)
        _ = fcntl(sp[0], F_SETFL, flags | O_NONBLOCK)

        let vpnIP    = key.vpnIP ?? "10.8.0.2"
        let settings = buildSettings(vpnIP: vpnIP, serverHost: key.serverHost, fullTunnel: fullTunnel)

        setTunnelNetworkSettings(settings) { [weak self] error in
            guard let self = self else { return }
            if let error = error {
                completionHandler(error)
                return
            }

            let host    = key.serverHost
            let port    = key.serverPort
            let sKeyArr = key.serverKey
            let pskArr  = key.psk
            let rustFd  = self.sp[1]

            // Decode optional base64-encoded mTLS cert (must be exactly 104 bytes)
            let certBytes: [UInt8]? = (cfg["mtlsCert"] as? String).flatMap {
                guard let data = Data(base64Encoded: $0), data.count == 104 else { return nil }
                return Array(data)
            }

            let thread = Thread {
                sKeyArr.withUnsafeBufferPointer { sKeyPtr in
                    let certCount = Int32(certBytes?.count ?? 0)
                    if let pskArr = pskArr {
                        pskArr.withUnsafeBufferPointer { pskPtr in
                            if let certBytes = certBytes {
                                certBytes.withUnsafeBufferPointer { certPtr in
                                    _ = aivpn_run_tunnel(rustFd, host, Int32(port),
                                                          sKeyPtr.baseAddress!, pskPtr.baseAddress!,
                                                          certPtr.baseAddress!, certCount,
                                                          nil, nil)
                                }
                            } else {
                                _ = aivpn_run_tunnel(rustFd, host, Int32(port),
                                                      sKeyPtr.baseAddress!, pskPtr.baseAddress!,
                                                      nil, 0, nil, nil)
                            }
                        }
                    } else {
                        if let certBytes = certBytes {
                            certBytes.withUnsafeBufferPointer { certPtr in
                                _ = aivpn_run_tunnel(rustFd, host, Int32(port),
                                                      sKeyPtr.baseAddress!, nil,
                                                      certPtr.baseAddress!, certCount,
                                                      nil, nil)
                            }
                        } else {
                            _ = aivpn_run_tunnel(rustFd, host, Int32(port),
                                                  sKeyPtr.baseAddress!, nil, nil, 0, nil, nil)
                        }
                    }
                }
            }
            thread.name = "aivpn-rust-tunnel"
            thread.qualityOfService = .userInitiated
            self.rustThread = thread
            thread.start()

            self.startBridge()
            completionHandler(nil)
        }
    }

    // MARK: - Stop
    //
    // Order matters:
    //   1. Set isStopped — packetFlow inbound loop will not re-queue after its
    //      current readPackets call returns.
    //   2. aivpn_stop_tunnel() — signals Rust to exit its event loop.
    //   3. Close sp[0] — causes Darwin.read in the outbound task to return EBADF,
    //      which breaks the loop and lets the Task finish.
    //   4. Close sp[1] — safe because Rust dup()d it; its copy is already being
    //      closed by step 2.
    //   5. Cancel / nil the outbound task.
    //   6. Call completionHandler — iOS tears down the extension process shortly
    //      after, so we don't need to join the Rust thread.

    override func stopTunnel(with reason: NEProviderStopReason,
                             completionHandler: @escaping () -> Void) {
        isStopped = true
        aivpn_stop_tunnel()
        if sp[0] >= 0 { Darwin.close(sp[0]); sp[0] = -1 }
        if sp[1] >= 0 { Darwin.close(sp[1]); sp[1] = -1 }
        outboundTask?.cancel()
        outboundTask = nil
        completionHandler()
    }

    // MARK: - IPC from main app

    override func handleAppMessage(_ messageData: Data,
                                   completionHandler: ((Data?) -> Void)?) {
        guard let json = try? JSONSerialization.jsonObject(with: messageData) as? [String: Any],
              let type = json["type"] as? String else {
            completionHandler?(nil)
            return
        }

        switch type {
        case "get_traffic":
            let up   = (NSNumber(value: aivpn_get_upload_bytes())).int64Value
            let down = (NSNumber(value: aivpn_get_download_bytes())).int64Value
            let resp: [String: Any] = [
                "upload":          up,
                "download":        down,
                "can_record":      canRecord,
                "recording_state": "idle",
            ]
            completionHandler?(try? JSONSerialization.data(withJSONObject: resp))

        default:
            completionHandler?(nil)
        }
    }

    // MARK: - Swift <-> Rust bridge

    private func startBridge() {
        let swiftFd = sp[0]
        let mtu = 1500

        // Inbound: packetFlow → sp[0] → Rust.
        // Uses isStopped rather than Task cancellation because readPackets uses
        // a callback that does not observe Swift concurrency cancellation.
        Task.detached { [weak self] in
            guard let self = self else { return }
            while !self.isStopped {
                let packets = await withCheckedContinuation { cont in
                    self.packetFlow.readPackets { pkts, _ in
                        cont.resume(returning: pkts)
                    }
                }
                guard !self.isStopped else { break }
                for pkt in packets {
                    pkt.withUnsafeBytes { buf in
                        _ = Darwin.write(swiftFd, buf.baseAddress!, buf.count)
                    }
                }
            }
        }

        // Outbound: sp[0] → packetFlow.
        // sp[0] is O_NONBLOCK so read() returns EAGAIN (no data) or EBADF (closed).
        outboundTask = Task.detached { [weak self] in
            guard let self = self else { return }
            var buf = [UInt8](repeating: 0, count: mtu + 4)
            while !Task.isCancelled {
                let n = Darwin.read(swiftFd, &buf, mtu + 4)
                if n > 0 {
                    let data = Data(buf[0..<n])
                    let af: NSNumber = (buf[0] >> 4 == 6)
                        ? NSNumber(value: AF_INET6) : NSNumber(value: AF_INET)
                    self.packetFlow.writePackets([data], withProtocols: [af])
                } else {
                    let e = errno
                    if e == EBADF || e == ENOTSOCK || e == EINVAL { break }
                    // EAGAIN — no data yet; yield and retry
                    try? await Task.sleep(nanoseconds: 500_000)
                }
            }
        }
    }

    // MARK: - Network settings

    private func buildSettings(vpnIP: String, serverHost: String,
                               fullTunnel: Bool) -> NEPacketTunnelNetworkSettings {
        let settings = NEPacketTunnelNetworkSettings(tunnelRemoteAddress: serverHost)
        settings.mtu = 1400

        let ipv4 = NEIPv4Settings(addresses: [vpnIP], subnetMasks: ["255.255.255.0"])
        if fullTunnel {
            ipv4.includedRoutes = [NEIPv4Route.default()]
        } else {
            ipv4.includedRoutes = [NEIPv4Route(destinationAddress: "10.8.0.0",
                                               subnetMask: "255.255.255.0")]
        }
        settings.ipv4Settings = ipv4

        if fullTunnel {
            let dns = NEDNSSettings(servers: ["10.8.0.1", "1.1.1.1"])
            settings.dnsSettings = dns
        }
        return settings
    }

    // MARK: - Helpers

    private func makeError(_ msg: String) -> NSError {
        NSError(domain: "com.aivpn.tunnel", code: -1,
                userInfo: [NSLocalizedDescriptionKey: msg])
    }
}

// MARK: - Minimal connection key parser (tunnel extension cannot import App target)

private struct TunnelConnectionKey {
    let serverHost: String
    let serverPort: UInt16
    let serverKey: [UInt8]
    let psk: [UInt8]?
    let vpnIP: String?
    let canRecord: Bool?

    init?(rawKey: String) {
        var b64 = rawKey
            .replacingOccurrences(of: "aivpn://", with: "")
            .replacingOccurrences(of: "-", with: "+")
            .replacingOccurrences(of: "_", with: "/")
        let rem = b64.count % 4
        if rem > 0 { b64 += String(repeating: "=", count: 4 - rem) }

        guard let data = Data(base64Encoded: b64),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let s    = json["s"] as? String else { return nil }

        let parts = s.split(separator: ":").map(String.init)
        guard parts.count >= 2, let port = UInt16(parts.last ?? "") else { return nil }
        serverHost = parts.dropLast().joined(separator: ":")
        serverPort = port

        guard let pkHex = json["k"] as? String,
              pkHex.count == 64,
              let keyBytes = Data(hexString: pkHex) else { return nil }
        serverKey = Array(keyBytes)

        if let pskHex = json["p"] as? String,
           pskHex.count == 64,
           let pskBytes = Data(hexString: pskHex) {
            psk = Array(pskBytes)
        } else {
            psk = nil
        }

        vpnIP     = json["i"] as? String
        canRecord = json["can_record"] as? Bool
    }
}

private extension Data {
    init?(hexString: String) {
        let chars = Array(hexString)
        guard hexString.count % 2 == 0 else { return nil }
        var bytes = [UInt8]()
        for i in stride(from: 0, to: chars.count, by: 2) {
            guard let hi = chars[i].hexDigitValue,
                  let lo = chars[i + 1].hexDigitValue else { return nil }
            bytes.append(UInt8(hi << 4 | lo))
        }
        self.init(bytes)
    }
}
