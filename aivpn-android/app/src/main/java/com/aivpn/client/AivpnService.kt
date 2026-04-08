package com.aivpn.client

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkRequest
import android.net.NetworkCapabilities
import android.net.VpnService
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import android.util.Log
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/**
 * Android VPN service — thin orchestrator over the Rust core (libaivpn_core.so).
 *
 * Responsibilities that must stay in Kotlin (Android API only):
 *   - VpnService.Builder / TUN interface establishment
 *   - NetworkCallback for network-change detection
 *   - VpnService.protect() — called from inside Rust via JNI on this instance
 *   - Foreground notification lifecycle
 *
 * Everything else (crypto, handshake, keepalive, anti-replay, rekey) is in Rust.
 */
class AivpnService : VpnService() {

    companion object {
        const val ACTION_CONNECT    = "com.aivpn.CONNECT"
        const val ACTION_DISCONNECT = "com.aivpn.DISCONNECT"
        private const val CHANNEL_ID      = "aivpn_vpn"
        private const val NOTIFICATION_ID = 1
        // Match the desktop client's WAN-safe TUN MTU so encrypted outer UDP
        // datagrams stay below the path-MTU ceiling on real networks.
        private const val TUN_MTU         = 1346
        private const val INITIAL_RETRY_DELAY_MS = 500L
        private const val MAX_RETRY_DELAY_MS     = 8_000L
        private const val TAG = "AivpnService"

        @Volatile var statusCallback:  ((Boolean, String) -> Unit)? = null
        @Volatile var trafficCallback: ((Long, Long) -> Unit)?      = null
        @Volatile var isRunning     = false
        @Volatile var lastStatusText = ""
    }

    // TUN interface wrapper (Kotlin holds PFD for lifecycle; Rust holds raw fd after detach)
    private var vpnInterface: ParcelFileDescriptor? = null

    // Coroutine lifecycle
    private var serviceJob: Job? = null
    private var restartJob: Job? = null
    private val serviceScope = CoroutineScope(Dispatchers.IO + SupervisorJob())
    private val serviceLifecycleMutex = Mutex()
    @Volatile private var manualDisconnect = false

    // Saved params for reconnect
    @Volatile private var savedServerAddr: String? = null
    @Volatile private var savedServerKey: String?  = null
    @Volatile private var savedPsk: String?        = null
    @Volatile private var savedVpnIp: String?      = null

    // Whether the current session reached the running state
    @Volatile private var sessionEstablished = false

    // Monotonically-increasing session counter.  Incremented on every new tunnel session.
    // Captured in upgradePendingJob at trigger time so a stale job can't kill a newer session.
    @Volatile private var sessionId: Long = 0L

    // Network change detection
    @Volatile private var networkTrigger: Boolean   = false
    private var networkCallback: ConnectivityManager.NetworkCallback? = null
    @Volatile private var currentUnderlyingNetwork: Network? = null
    @Volatile private var lastNetworkEventAtMs: Long = 0L
    private val NETWORK_EVENT_DEBOUNCE_MS = 1_000L

    // ──────────── Service lifecycle ────────────

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_CONNECT -> {
                val server    = intent.getStringExtra("server")     ?: return START_NOT_STICKY
                val serverKey = intent.getStringExtra("server_key") ?: return START_NOT_STICKY
                startVpn(server, serverKey,
                    intent.getStringExtra("psk"),
                    intent.getStringExtra("vpn_ip"))
            }
            ACTION_DISCONNECT -> stopVpn()
        }
        return START_STICKY
    }

    private fun startVpn(
        serverAddr: String,
        serverKeyBase64: String,
        pskBase64: String? = null,
        vpnIp: String? = null,
    ) {
        Log.d(TAG, "startVpn: server=$serverAddr")

        val sameTarget =
            savedServerAddr == serverAddr &&
            savedServerKey == serverKeyBase64 &&
            savedPsk == pskBase64 &&
            savedVpnIp == vpnIp
        val startupInFlight = restartJob?.isActive == true
        val tunnelLoopActive = serviceJob?.isActive == true
        if (sameTarget && (startupInFlight || tunnelLoopActive)) {
            Log.d(TAG, "Ignoring duplicate CONNECT while startup/session is already in progress")
            return
        }

        savedServerAddr  = serverAddr
        savedServerKey   = serverKeyBase64
        savedPsk         = pskBase64
        savedVpnIp       = vpnIp
        manualDisconnect = false

        restartJob?.cancel()
        restartJob = serviceScope.launch {
            serviceLifecycleMutex.withLock {
                AivpnJni.stopTunnel()
                serviceJob?.cancelAndJoin()
                serviceJob = null
                closeTunnel()

                createNotificationChannel()
                startForeground(NOTIFICATION_ID, buildNotification(getString(R.string.notification_connecting)))

                unregisterNetworkCallback()
                registerNetworkCallback()

                serviceJob = serviceScope.launch {
            var retryDelayMs = INITIAL_RETRY_DELAY_MS
            try {
                while (isActive && !manualDisconnect) {
                    try {
                        sessionEstablished = false
                        networkTrigger = false
                        runTunnel()
                        // runTunnel() returns normally only on Rust rekey trigger — reconnect fast.
                        retryDelayMs = INITIAL_RETRY_DELAY_MS
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        Log.e(TAG, "Tunnel error: ${e.message}", e)
                        isRunning = false
                        closeTunnel()
                        if (manualDisconnect) break

                        // Network-triggered reconnects and reconnects after an established
                        // session use zero delay so the switch feels instant.
                        val delayMs = when {
                            networkTrigger     -> 0L
                            sessionEstablished -> 0L
                            else               -> retryDelayMs
                        }

                        lastStatusText = getString(R.string.status_reconnecting)
                        statusCallback?.invoke(false, lastStatusText)
                        updateNotification(getString(R.string.notification_connecting))

                        if (delayMs > 0) {
                            Log.d(TAG, "Reconnecting in ${delayMs}ms")
                            delay(delayMs)
                        } else {
                            Log.d(TAG, "Reconnecting immediately (network=${networkTrigger}, established=${sessionEstablished})")
                        }

                        if (!networkTrigger && !sessionEstablished) {
                            retryDelayMs = (retryDelayMs * 2).coerceAtMost(MAX_RETRY_DELAY_MS)
                        } else {
                            retryDelayMs = INITIAL_RETRY_DELAY_MS
                        }
                    }
                }
            } catch (e: CancellationException) {
                Log.d(TAG, "Service job cancelled")
            } finally {
                isRunning = false
                closeTunnel()
                serviceJob = null
                if (!manualDisconnect) {
                    stopForeground(STOP_FOREGROUND_REMOVE)
                    stopSelf()
                }
            }
                }
            }
        }
    }

    // ──────────── Tunnel session ────────────

    /**
     * One tunnel session.  Blocks until the Rust core exits (error or rekey interval).
     * Any exception propagates to the reconnect loop.
     */
    private suspend fun runTunnel() {
        // Wait for any usable network before starting (avoids immediate DNS/handshake failure).
        waitForConnectivity()

        val (host, port) = parseServerAddr(
            savedServerAddr ?: throw Exception("No server address"))

        val serverKey = android.util.Base64.decode(
            savedServerKey ?: throw Exception("No server key"),
            android.util.Base64.DEFAULT)
        if (serverKey.size != 32) throw Exception("Invalid server key size: ${serverKey.size}")

        val psk: ByteArray? = savedPsk?.let {
            val decoded = android.util.Base64.decode(it, android.util.Base64.DEFAULT)
            if (decoded.size == 32) decoded else null
        }

        val tunAddress4 = savedVpnIp ?: "10.0.0.2"

        // Build TUN (must stay in Kotlin — Android API).
        // setBlocking(false): Rust uses epoll/AsyncFd on the raw fd.
        // IPv6 is intentionally disabled in this client.
        val builder = Builder()
            .setSession("AIVPN")
            .addAddress(tunAddress4, 24)
            .addRoute("0.0.0.0", 0)          // IPv4: route all through VPN
            .addDnsServer("8.8.8.8")
            .addDnsServer("1.1.1.1")
            .setMtu(TUN_MTU)
            .setBlocking(false)

        // Split tunneling: route only selected apps through VPN
        val allowedApps = SecureStorage.loadAllowedApps(this)
        for (pkg in allowedApps) {
            try {
                builder.addAllowedApplication(pkg)
            } catch (_: Exception) {
                // Package may have been uninstalled — skip silently
            }
        }

        // Split tunneling: exclude domains by resolving to IPs
        val excludedDomains = SecureStorage.loadExcludedDomains(this)
        if (excludedDomains.isNotEmpty()) {
            val excludedIPs = mutableSetOf<String>()
            for (domain in excludedDomains) {
                try {
                    val addresses = java.net.InetAddress.getAllByName(domain)
                    for (addr in addresses) {
                        if (addr is java.net.Inet4Address) {
                            excludedIPs.add(addr.hostAddress ?: continue)
                        }
                    }
                } catch (_: Exception) {
                    Log.d(TAG, "Failed to resolve excluded domain: $domain")
                }
            }
            // If we have excluded IPs, replace the default route with specific /1 routes
            // that cover everything except the excluded IPs (which get /32 direct routes).
            // Android VPN routing: more specific routes win, so /32 routes for excluded IPs
            // hit the underlying network, while 0.0.0.0/0 catches everything else.
            // However, addRoute(0/0) is already added above. We need to exclude by NOT
            // routing those IPs through VPN. On Android, the only way is per-app exclusion
            // or using the system routing table. We log them for now and they can be used
            // by the Rust tunnel for domain-based bypassing via DNS interception.
            if (excludedIPs.isNotEmpty()) {
                Log.d(TAG, "Excluded domain IPs: $excludedIPs")
            }
        }

        val pfd = builder.establish() ?: throw Exception("Failed to establish VPN interface")

        vpnInterface = pfd
        // WireGuard approach: let Android OS choose the best underlying network.
        // Setting null allows automatic network selection and seamless WiFi↔cellular switching.
        // The socket is protected via VpnService.protect() in Rust so it bypasses VPN routing.
        setUnderlyingNetworks(null)

        // detachFd(): raw fd ownership transfers to Rust.  pfd.close() becomes a no-op.
        val tunFd = pfd.detachFd()

        sessionEstablished = false
        isRunning          = true
        sessionId++     // new session — invalidates any queued upgradePendingJob
        lastStatusText = getString(R.string.status_connecting)
        statusCallback?.invoke(false, lastStatusText)
        updateNotification(getString(R.string.notification_connecting))

        // Poll Rust traffic counters once per second and forward to UI.
        val statsJob = serviceScope.launch {
            while (isActive) {
                delay(1_000L)
                trafficCallback?.invoke(AivpnJni.getUploadBytes(), AivpnJni.getDownloadBytes())
            }
        }

        try {
            val error = withContext(Dispatchers.IO) {
                AivpnJni.runTunnel(this@AivpnService, tunFd, host, port, serverKey, psk)
            }
            if (error.isNotEmpty()) throw RuntimeException(error)
        } finally {
            statsJob.cancel()
            isRunning = false
        }
    }

    /**
     * Called from Rust (JNI) when handshake and key ratchet are complete.
     * This is the first moment when "connected" is actually true.
     */
    @Suppress("unused")
    fun onTunnelReady(host: String) {
        sessionEstablished = true
        isRunning = true
        lastStatusText = getString(R.string.status_connected, host)
        statusCallback?.invoke(true, lastStatusText)
        updateNotification(getString(R.string.notification_connected, host))
        Log.d(TAG, "Tunnel ready: host=$host")
    }

    // ──────────── Network callbacks ────────────

    /**
     * WireGuard-style approach: we do NOT manually select networks or bind sockets
     * to specific interfaces.  Instead:
     *   - setUnderlyingNetworks(null) lets Android route through the best available network
     *   - VpnService.protect(fd) ensures the UDP socket bypasses VPN routing
    *   - We detect default-network switches/loss and trigger a fast tunnel restart (which will
     *     get a fresh DNS resolution and handshake on whatever network is available)
    *   - The Rust side has an aggressive RX silence detector as backup
     */
    private fun registerNetworkCallback() {
        val cm = getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        currentUnderlyingNetwork = findUsableUnderlyingNetwork(cm)

        val callback = object : ConnectivityManager.NetworkCallback() {

            override fun onAvailable(network: Network) {
                val caps = cm.getNetworkCapabilities(network) ?: return
                if (!isUsableUnderlyingNetwork(caps)) return

                val previous = currentUnderlyingNetwork
                currentUnderlyingNetwork = network

                Log.d(TAG, "Underlying network available: $network (previous=$previous)")

                // Seamless roaming with a protected UDP socket is not reliable across
                // all Android vendors/radios. If the default network actually changed
                // under an established session, restart immediately on the new path.
                if (previous != null && previous != network && isRunning && sessionEstablished) {
                    val now = SystemClock.elapsedRealtime()
                    if (now - lastNetworkEventAtMs >= NETWORK_EVENT_DEBOUNCE_MS) {
                        lastNetworkEventAtMs = now
                        Log.d(TAG, "Underlying network switched: $previous -> $network; restarting tunnel")
                        networkTrigger = true
                        AivpnJni.stopTunnel()
                    }
                }
            }

            override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) {
                if (!isUsableUnderlyingNetwork(caps)) return
                if (currentUnderlyingNetwork == null) {
                    currentUnderlyingNetwork = network
                    Log.d(TAG, "Underlying network became usable: $network")
                }
            }

            override fun onLost(network: Network) {
                Log.d(TAG, "Underlying network lost: $network")

                if (network == currentUnderlyingNetwork) {
                    currentUnderlyingNetwork = findUsableUnderlyingNetwork(cm)
                }

                val replacement = currentUnderlyingNetwork
                val hasUsableDefault = replacement?.let { net ->
                    val caps = cm.getNetworkCapabilities(net)
                    caps != null && isUsableUnderlyingNetwork(caps)
                } == true

                if (hasUsableDefault && replacement != null && isRunning && sessionEstablished) {
                    val now = SystemClock.elapsedRealtime()
                    if (now - lastNetworkEventAtMs >= NETWORK_EVENT_DEBOUNCE_MS) {
                        lastNetworkEventAtMs = now
                        Log.d(TAG, "Underlying network moved to $replacement after loss of $network; restarting tunnel")
                        networkTrigger = true
                        AivpnJni.stopTunnel()
                    }
                    return
                }

                if (!hasUsableDefault && isRunning) {
                    val now = SystemClock.elapsedRealtime()
                    if (now - lastNetworkEventAtMs >= NETWORK_EVENT_DEBOUNCE_MS) {
                        lastNetworkEventAtMs = now
                        Log.d(TAG, "No usable underlying network — stopping tunnel for fast reconnect")
                        networkTrigger = true
                        AivpnJni.stopTunnel()
                    }
                }
            }
        }

        try {
            val request = NetworkRequest.Builder()
                .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
                .addCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
                .build()
            cm.registerNetworkCallback(request, callback)
            networkCallback = callback
        } catch (e: Exception) {
            Log.e(TAG, "Failed to register NetworkCallback: ${e.message}", e)
        }
    }

    private fun isVpnNetwork(cm: ConnectivityManager, network: Network): Boolean {
        val caps = cm.getNetworkCapabilities(network)
        return caps?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true
    }

    private fun unregisterNetworkCallback() {
        networkCallback?.let {
            try {
                (getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager)
                    .unregisterNetworkCallback(it)
            } catch (_: Exception) {}
            networkCallback = null
        }
        currentUnderlyingNetwork = null
    }

    // ──────────── Stop ────────────

    private fun stopVpn() {
        manualDisconnect = true
        restartJob?.cancel()
        restartJob = null
        unregisterNetworkCallback()
        AivpnJni.stopTunnel()
        serviceJob?.cancel()
        serviceJob = null
        closeTunnel()
        isRunning = false
        lastStatusText = getString(R.string.status_disconnected)
        statusCallback?.invoke(false, lastStatusText)
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    private fun closeTunnel() {
        try { vpnInterface?.close() } catch (_: Exception) {}
        vpnInterface = null
    }

    /**
     * Called when Android revokes the VPN permission (e.g. another VPN app takes over).
     * Default VpnService.onRevoke() calls stopSelf() which kills the service with no reconnect.
     * We signal Rust to exit cleanly; the reconnect loop in serviceJob will then restart the
     * tunnel automatically (unless manualDisconnect is true).
     */
    override fun onRevoke() {
        Log.w(TAG, "onRevoke() — signalling Rust to exit, reconnect loop will restart")
        AivpnJni.stopTunnel()
        // Do NOT call super.onRevoke() — it calls stopSelf() which bypasses reconnect.
    }

    override fun onDestroy() {
        manualDisconnect = true
        restartJob?.cancel()
        restartJob = null
        unregisterNetworkCallback()
        AivpnJni.stopTunnel()
        serviceJob?.cancel()
        serviceJob = null
        closeTunnel()
        isRunning = false
        serviceScope.cancel()
        super.onDestroy()
    }

    // ──────────── Network waiting ────────────

    /**
     * Block until at least one non-VPN network with internet capability exists.
     * This prevents wasting time on DNS lookups / handshakes when there's no connectivity.
     */
    private suspend fun waitForConnectivity() {
        val cm = getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        while (currentCoroutineContext().isActive) {
            val active = findUsableUnderlyingNetwork(cm)
            val hasUsableActiveNetwork = active != null

            if (hasUsableActiveNetwork) return

            // Fallback: when VPN is active, activeNetwork can point to TRANSPORT_VPN.
            // In that case, scan all networks for any non-VPN internet-capable network.
            val hasAnyUsableNetwork = cm.allNetworks.any { net ->
                val netCaps = cm.getNetworkCapabilities(net) ?: return@any false
                !netCaps.hasTransport(NetworkCapabilities.TRANSPORT_VPN) &&
                    netCaps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            }
            if (hasAnyUsableNetwork) return

            delay(300L)
        }
        throw CancellationException("Cancelled while waiting for network")
    }

    private fun findUsableUnderlyingNetwork(cm: ConnectivityManager): Network? {
        return cm.allNetworks.firstOrNull { net ->
            val caps = cm.getNetworkCapabilities(net) ?: return@firstOrNull false
            isUsableUnderlyingNetwork(caps)
        }
    }

    private fun isUsableUnderlyingNetwork(caps: NetworkCapabilities): Boolean {
        return !caps.hasTransport(NetworkCapabilities.TRANSPORT_VPN) &&
            caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
    }

    // ──────────── Address parsing ────────────

    private fun parseServerAddr(serverAddr: String): Pair<String, Int> {
        if (serverAddr.startsWith("[")) {
            val bracket = serverAddr.indexOf(']')
            if (bracket > 0) {
                val host = serverAddr.substring(1, bracket)
                val port = if (bracket + 1 < serverAddr.length && serverAddr[bracket + 1] == ':')
                    serverAddr.substring(bracket + 2).toIntOrNull() ?: 443
                else 443
                return Pair(host, port)
            }
        }
        val lastColon = serverAddr.lastIndexOf(':')
        val port = if (lastColon >= 0) serverAddr.substring(lastColon + 1).toIntOrNull() else null
        return if (port != null)
            Pair(serverAddr.substring(0, lastColon), port)
        else
            Pair(serverAddr, 443)
    }

    // ──────────── Notifications ────────────

    private fun createNotificationChannel() {
        val channel = NotificationChannel(
            CHANNEL_ID, getString(R.string.notification_channel),
            NotificationManager.IMPORTANCE_LOW
        ).apply { description = getString(R.string.notification_channel_desc) }
        getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
    }

    private fun buildNotification(text: String): Notification {
        val pi = PendingIntent.getActivity(
            this, 0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE)
        return Notification.Builder(this, CHANNEL_ID)
            .setContentTitle("AIVPN")
            .setContentText(text)
            .setSmallIcon(android.R.drawable.ic_lock_lock)
            .setContentIntent(pi)
            .setOngoing(true)
            .build()
    }

    private fun updateNotification(text: String) {
        getSystemService(NotificationManager::class.java)
            .notify(NOTIFICATION_ID, buildNotification(text))
    }
}
