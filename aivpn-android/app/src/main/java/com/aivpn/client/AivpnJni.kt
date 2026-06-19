package com.aivpn.client

import android.net.VpnService

/**
 * JNI bridge to the native Rust core (libaivpn_core.so).
 *
 * The library is cross-compiled for arm64-v8a / armeabi-v7a / x86_64 and placed in
 * app/src/main/jniLibs/ by build-rust-android.sh.
 */
object AivpnJni {

    init {
        System.loadLibrary("aivpn_core")
    }

    /**
     * Runs a full VPN tunnel session on the calling thread (blocks until done).
     *
     * @param vpnService  The VpnService instance — used to call `protect(int)` on the UDP socket.
     * @param tunFd       Raw file descriptor from [android.os.ParcelFileDescriptor.detachFd].
     *                    Rust takes ownership and will close it when the session ends.
     * @param serverHost  Server hostname or IP.
     * @param serverPort  Server UDP port.
     * @param serverKey   32-byte server X25519 public key.
     * @param psk         32-byte pre-shared key or `null`.
     * @return            Empty string on a clean rekey-triggered exit, error message otherwise.
     */
    /**
     * adaptiveLevel: 0=Off, 1=Light (keepalive 6s), 2=Aggressive (4s), 3=Satellite (15s).
     * The level controls keepalive interval and FEC group size.
     */
    external fun runTunnel(
        vpnService: VpnService,
        tunFd: Int,
        serverHost: String,
        serverPort: Int,
        serverKey: ByteArray,
        psk: ByteArray?,
        mtlsCert: ByteArray?,
        adaptiveLevel: Int,
        staticPrivkey: ByteArray?,
    ): String

    /**
     * Closes the protected UDP socket so the tunnel loop exits immediately.
     * Safe to call from any thread, including the NetworkCallback.
     */
    external fun stopTunnel()

    /**
     * Clears the STOP_PENDING flag set by [stopTunnel] when no session was active.
     * Must be called in the restartJob after [Job.cancelAndJoin] and before launching
     * the new connection so the intentional new session is not immediately stopped.
     */
    external fun clearPendingStop()

    /** Total bytes written to the server UDP socket in the current session. */
    external fun getUploadBytes(): Long

    /** Total bytes written to the TUN interface in the current session. */
    external fun getDownloadBytes(): Long

    /** Connection quality score 0–100 from last KeepaliveAck RTT. 0 = no data yet. */
    external fun getQualityScore(): Int

    /** Adaptive level hint from server (0–3). 0 = no hint received. Takes effect on next reconnect. */
    external fun getAdaptiveLevelHint(): Int

    /** Send RecordingStart to the server. Returns 1 if queued, 0 if no active session. */
    external fun startRecording(serviceName: String): Int

    /** Send RecordingStop to the server. No-op if no active session. */
    external fun stopRecording()
}
