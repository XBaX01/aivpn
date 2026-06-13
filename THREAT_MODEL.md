# AIVPN Threat Model

This document describes the security properties of AIVPN, the adversary model it is designed against, and the known limitations of the current implementation.

---

## 1. Goals

AIVPN is designed to provide:

1. **Confidentiality** — payload content is not visible to a passive observer on the network path.
2. **Traffic-analysis resistance** — packet sizes, inter-arrival times, and connection patterns are disguised as known benign application traffic (WebRTC, QUIC, etc.).
3. **Active-probing resistance** — a server does not respond recognizably to unsolicited or malformed probe packets.
4. **Forward secrecy** — compromise of long-term keys does not decrypt previously captured sessions.
5. **Availability under censorship** — the system detects when a mask profile is fingerprinted by DPI and rotates to a fresh one automatically.

---

## 2. Adversary Model

### 2.1 In-scope adversaries

| Adversary | Capability | Threat |
|-----------|-----------|--------|
| **Passive ISP/censor** | Reads all packets between client and server | Traffic analysis, protocol identification |
| **Active prober** | Sends crafted packets to the server endpoint | Fingerprinting by server response behavior |
| **DPI appliance** | Stateful analysis of flow characteristics | Mask fingerprinting, throttling |
| **Network observer** | Correlates timing of flows across vantage points | Traffic correlation |
| **Stolen server key** | Off-line; gains static server private key | Cannot decrypt past sessions (PFS) |
| **Compromised server** | Full access to server memory and disk | Ongoing sessions exposed; past sessions protected by PFS |

### 2.2 Out-of-scope adversaries

| Adversary | Reason out of scope |
|-----------|-------------------|
| **Compromised client endpoint** | If the OS or application running the client is controlled by the adversary, no VPN protocol can help. |
| **Global passive adversary** | Traffic correlation between the client's ISP and the server's ISP is a hard problem; AIVPN does not claim resistance to a global passive adversary. |
| **Physical server seizure** | Best handled by full-disk encryption and key destruction procedures outside the scope of this protocol. |
| **DNS interception** | Hostname resolution for the server address happens before the VPN is established; protect DNS separately. |

---

## 3. Cryptographic Design

### 3.1 Key exchange

- **Algorithm:** X25519 Diffie-Hellman (ephemeral client keypair, static server public key).
- **PSK:** An optional 32-byte pre-shared key is mixed into key derivation, providing a second authentication factor.
- **Derived keys:** BLAKE3-based KDF produces `session_key`, `tag_secret`, and `nonce_suffix` from the DH output.

### 3.2 Authenticated encryption

- **Algorithm:** ChaCha20-Poly1305 (IETF variant, 96-bit nonce).
- **Nonce construction:** `counter (8 bytes) || nonce_suffix (4 bytes)`. The counter is monotonically increasing per session; reuse is prevented by design.

### 3.3 Session tags (O(1) lookup without session IDs)

Every packet carries an 8-byte *resonance tag* derived from the current timestamp and `tag_secret`. The server maintains a 256-entry sliding window per session to allow out-of-order delivery. Tags are non-guessable without `tag_secret`.

**Anti-replay:** Tags outside the acceptance window (`DEFAULT_WINDOW_MS = 10 000 ms`) are dropped. The XDP early filter enforces the same window at NIC level.

### 3.4 Perfect Forward Secrecy (PFS ratchet)

After the initial session, the server sends a `ServerHello` with a fresh ephemeral public key. The client computes a new DH secret, derives ratcheted keys, and begins sending on the new keys immediately. The old keys are retained briefly for in-flight packets, then discarded.

**Property:** An attacker who records the ciphertext stream and later obtains the server's static private key cannot decrypt any session that has completed at least one ratchet step.

---

## 4. Traffic Analysis Resistance

### 4.1 Mask mimicry

Outbound packets are shaped to match a selected traffic profile (`MaskProfile`):
- **Header injection:** Synthetic application-layer headers are prepended to each packet.
- **Size shaping:** Payloads are padded to match the target size distribution.
- **Timing shaping (IAT):** The mimicry engine enforces inter-arrival time distributions from the mask profile.
- **FSM-driven state transitions:** Traffic evolves through a finite-state machine matching the modeled application's conversation phases.

### 4.2 Neural Resonance (automated mask rotation)

A per-mask MLP (~66 KB, deterministically derived from the mask signature vector) monitors live traffic statistics. When the reconstruction error (MSE) exceeds `compromised_threshold = 0.35`, the server triggers a mask rotation and pushes the new mask to connected clients.

Features monitored: packet size distribution, IAT statistics, entropy, burst patterns, packet direction ratio, and IAT periodicity.

**Rotation cooldown:** 60 seconds between rotations prevents oscillation under sustained active probing.

### 4.3 Active-probing resistance

- The server does not respond to packets that fail tag validation.
- Tag validation requires knowledge of `tag_secret`, which is only derivable after a successful DH handshake.
- Unsolicited probes receive no response, making the server's protocol indistinguishable from a UDP echo service or game server to an outside observer.

---

## 5. Kill-Switch & Leak Protection

When `--kill-switch` is active, the client installs firewall rules that drop all outbound traffic except:
- Traffic on the VPN TUN interface.
- Traffic to the physical VPN server IP (so the tunnel can be re-established).
- Loopback traffic.

**Implementation:**
- **Linux:** nftables table `aivpn_ks` with drop policy; iptables chain `AIVPN_KS` as fallback.
- **macOS:** pfctl anchor `aivpn_ks`.
- **Windows:** Windows Firewall rules via `netsh advfirewall`.

Rules persist across unexpected process death (SIGKILL) by design — the user remains protected until they explicitly run `aivpn-client kill-switch clear`.

**No shell injection:** All firewall rule arguments are passed as distinct `argv` elements; no string interpolation through a shell.

**macOS secure write:** pfctl anchor rules are written to `/var/run/aivpn/` (mode `0700`) using `O_NOFOLLOW | O_CREAT_NEW | mode(0o600)` to prevent symlink attacks against world-writable directories.

---

## 6. XDP Early Filter

When `xdp_prog.o` is installed, the client attaches an XDP BPF program to the physical NIC (the default-route interface). The filter runs at NIC RX level, before socket buffer allocation:

- Drops UDP packets shorter than 26 bytes (minimum valid AIVPN payload).
- Drops UDP packets whose resonance tag timestamp falls outside the acceptance window (default ±10 s from `bpf_ktime_get_ns()`).
- All other packets pass through to the normal network stack unchanged.

**Effect:** Volumetric DDoS packets with random payloads are dropped before they consume kernel networking resources. Legitimate traffic is unaffected.

**Failure mode:** If `xdp_prog.o` is absent or attachment fails, the VPN operates normally without XDP — it is a best-effort optimization, not a security requirement.

---

## 7. Known Limitations

| Limitation | Notes |
|-----------|-------|
| **XDP filter is IPv4-only** | IPv6 packets pass through XDP unconditionally. For IPv6-only deployments this reduces DDoS protection at NIC level. |
| **Traffic correlation** | Timing correlations between the client ISP and server ISP may be exploitable by a sufficiently resourced adversary. |
| **Mask quality** | A poorly crafted mask (low confidence score) may be distinguishable from real traffic by a trained classifier. Use `--validate-mask` before deploying custom profiles. |
| **Single-hop** | AIVPN is a single-hop VPN; the server knows the client's real IP. Use in combination with a trusted exit node if anonymity is required. |
| **PSK distribution** | The PSK embedded in the connection key must be distributed securely. Compromise of the connection key string allows impersonation. |
| **Kill-switch on SIGKILL only persists until reboot** | The firewall rules are loaded in the running kernel/firewall; they do not survive a reboot. This is intentional — a rebooted system starts clean. |

---

## 8. Reporting Security Issues

Please report security vulnerabilities by email to **vladislav@minakov.pro** with subject `[AIVPN SECURITY]`. Do not open a public GitHub issue for vulnerability reports.

Include:
- Affected version / commit hash.
- A description of the vulnerability and potential impact.
- Reproduction steps or proof-of-concept if available.

We aim to respond within 72 hours and provide a fix within 14 days for critical issues.
