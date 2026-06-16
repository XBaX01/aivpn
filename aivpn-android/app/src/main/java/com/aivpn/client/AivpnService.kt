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
import android.os.Build
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import android.util.Base64
import android.util.Log
import kotlinx.coroutines.*
import org.json.JSONObject
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.net.Inet4Address
import java.net.InetAddress

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
        private const val DEFAULT_TUN_MTU = 1346
        private const val ADAPTIVE_TUN_MTU = 1200
        private const val LEGACY_PREFIX_LEN = 24
        private const val INITIAL_RETRY_DELAY_MS = 500L
        private const val MAX_RETRY_DELAY_MS     = 8_000L
        // Android reshuffles underlying network IDs for 5-10s after VPN comes up.
        // 15s covers even slow devices without delaying genuine network-switch detection.
        private const val TAG = "AivpnService"

        @Volatile var statusCallback:  ((Boolean, String) -> Unit)? = null
        @Volatile var trafficCallback: ((Long, Long) -> Unit)?      = null
        @Volatile var tileCallback:    (() -> Unit)?                = null
        @Volatile var isRunning     = false
        @Volatile var isServiceActive = false
        @Volatile var lastStatusText = ""

        /** Weak reference to the live service instance, used for socket protection only. */
        @Volatile var instance: AivpnService? = null
    }

    // TUN interface wrapper kept open across reconnects so Android does not tear down
    // the device-level VPN interface between Rust tunnel restarts.
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
    @Volatile private var savedMtlsCert: ByteArray? = null
    @Volatile private var savedVpnIp: String?      = null
    @Volatile private var savedServerVpnIp: String? = null
    @Volatile private var savedVpnPrefixLen: Int = LEGACY_PREFIX_LEN
    @Volatile private var savedVpnMtu: Int = DEFAULT_TUN_MTU

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
    // Android reshuffles underlying network IDs when VPN comes up; ignore churn for this window.
    @Volatile private var postConnectUntilMs: Long = 0L
    private val POST_CONNECT_COOLDOWN_MS = 15_000L

    // ──────────── Service lifecycle ────────────

    override fun onCreate() {
        super.onCreate()
        instance = this
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_CONNECT -> {
                // Read only the profile ID from the Intent.  The actual server key
                // and PSK are loaded from EncryptedSharedPreferences inside the
                // service so they never travel through IPC as plaintext extras.
                val profileId = intent.getStringExtra("profile_id") ?: return START_NOT_STICKY
                loadAndStartVpnFromProfile(profileId)
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
        serverVpnIp: String? = null,
        vpnPrefixLen: Int = LEGACY_PREFIX_LEN,
        vpnMtu: Int = DEFAULT_TUN_MTU,
        mtlsCert: ByteArray? = null,
    ) {
        Log.d(TAG, "startVpn: server=$serverAddr")

        val normalizedPrefixLen = vpnPrefixLen.coerceIn(1, 30)
        val normalizedMtu = vpnMtu.coerceAtLeast(576)

        val sameTarget =
            savedServerAddr == serverAddr &&
            savedServerKey == serverKeyBase64 &&
            savedPsk == pskBase64 &&
            savedVpnIp == vpnIp &&
            savedServerVpnIp == serverVpnIp &&
            savedVpnPrefixLen == normalizedPrefixLen &&
            savedVpnMtu == normalizedMtu &&
            (savedMtlsCert == null && mtlsCert == null ||
             savedMtlsCert != null && mtlsCert != null && savedMtlsCert.contentEquals(mtlsCert))
        val startupInFlight = restartJob?.isActive == true
        val tunnelLoopActive = serviceJob?.isActive == true
        if (sameTarget && (startupInFlight || tunnelLoopActive)) {
            Log.d(TAG, "Ignoring duplicate CONNECT while startup/session is already in progress")
            return
        }

        savedServerAddr  = serverAddr
        savedServerKey   = serverKeyBase64
        savedPsk         = pskBase64
        savedMtlsCert    = mtlsCert
        savedVpnIp       = vpnIp
        savedServerVpnIp = serverVpnIp
        savedVpnPrefixLen = normalizedPrefixLen
        savedVpnMtu = normalizedMtu
        manualDisconnect = false
        isServiceActive = true

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
                        closeTunnel()
                        // runTunnel() returns normally only on Rust rekey trigger — reconnect fast.
                        retryDelayMs = INITIAL_RETRY_DELAY_MS
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        Log.e(TAG, "Tunnel error: ${e.message}", e)
                        isRunning = false
                        if (manualDisconnect) break
                        closeTunnel()

                        // Network-triggered reconnects and reconnects after an established
                        // session use zero delay so the switch feels instant.
                        val delayMs = when {
                            networkTrigger     -> 0L
                            sessionEstablished -> 0L
                            else               -> retryDelayMs
                        }

                        lastStatusText = getString(R.string.status_reconnecting)
                        val cb0 = statusCallback; cb0?.invoke(false, lastStatusText)
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
                serviceJob = null
                if (!manualDisconnect) {
                    isServiceActive = false
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

        ensureVpnInterface()
        val activeTun = vpnInterface ?: throw Exception("VPN interface is not available")
        // WireGuard approach: let Android OS choose the best underlying network.
        // Setting null allows automatic network selection and seamless WiFi↔cellular switching.
        // The socket is protected via VpnService.protect() in Rust so it bypasses VPN routing.
        setUnderlyingNetworks(null)

        // Keep the original ParcelFileDescriptor open in the service so Android keeps the
        // device VPN active across reconnects. Rust receives only the borrowed fd number
        // and duplicates it on its side, so it never closes an Android-owned descriptor.
        val tunFd = activeTun.fd

        sessionEstablished = false
        isRunning          = true
        sessionId++     // new session — invalidates any queued upgradePendingJob
        lastStatusText = getString(R.string.status_connecting)
        val cb1 = statusCallback; cb1?.invoke(false, lastStatusText)
        updateNotification(getString(R.string.notification_connecting))

        // Poll Rust traffic counters once per second and forward to UI.
        val statsJob = serviceScope.launch {
            while (isActive) {
                delay(1_000L)
                val tcb = trafficCallback; tcb?.invoke(AivpnJni.getUploadBytes(), AivpnJni.getDownloadBytes())
            }
        }

        try {
            val error = withContext(Dispatchers.IO) {
                AivpnJni.runTunnel(this@AivpnService, tunFd, host, port, serverKey, psk, savedMtlsCert, isAdaptiveEnabled())
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
        // postConnectUntilMs must be written BEFORE sessionEstablished — network callbacks
        // check sessionEstablished first and then read postConnectUntilMs; if the order were
        // reversed a callback could see sessionEstablished=true while postConnectUntilMs=0.
        postConnectUntilMs = SystemClock.elapsedRealtime() + POST_CONNECT_COOLDOWN_MS
        sessionEstablished = true
        isRunning = true
        lastStatusText = getString(R.string.status_connected, host)
        val cb2 = statusCallback; cb2?.invoke(true, lastStatusText)
        val ticb0 = tileCallback; ticb0?.invoke()
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
                    if (now < postConnectUntilMs) {
                        // VPN just came up — Android re-registers network IDs; ignore this churn.
                        currentUnderlyingNetwork = network
                    } else if (now - lastNetworkEventAtMs >= NETWORK_EVENT_DEBOUNCE_MS) {
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
                    if (now >= postConnectUntilMs && now - lastNetworkEventAtMs >= NETWORK_EVENT_DEBOUNCE_MS) {
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

    private fun isAdaptiveEnabled(): Boolean =
        getSharedPreferences("aivpn_prefs", MODE_PRIVATE)
            .getBoolean("adaptive_enabled", false)

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
        isServiceActive = false
        restartJob?.cancel()
        restartJob = null
        unregisterNetworkCallback()
        AivpnJni.stopTunnel()
        serviceJob?.cancel()
        serviceJob = null
        closeTunnel()
        isRunning = false
        lastStatusText = getString(R.string.status_disconnected)
        val cb3 = statusCallback; cb3?.invoke(false, lastStatusText)
        val ticb1 = tileCallback; ticb1?.invoke()
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    private fun closeTunnel() {
        try { vpnInterface?.close() } catch (_: Exception) {}
        vpnInterface = null
    }

    /**
     * Called when Android revokes the VPN permission (e.g. another VPN app takes over).
     * Mark as manual disconnect so the reconnect loop does not attempt to reconnect, stop
     * the tunnel, and call super so Android tears down the VPN interface and service cleanly.
     */
    override fun onRevoke() {
        Log.w(TAG, "onRevoke() — OS revoked VPN permission, stopping permanently")
        manualDisconnect = true
        stopVpn()
        super.onRevoke()
    }

    override fun onDestroy() {
        manualDisconnect = true
        isServiceActive = false
        restartJob?.cancel()
        restartJob = null
        unregisterNetworkCallback()
        AivpnJni.stopTunnel()
        serviceJob?.cancel()
        serviceJob = null
        closeTunnel()
        isRunning = false
        serviceScope.cancel()
        statusCallback = null
        trafficCallback = null
        tileCallback = null
        instance = null
        super.onDestroy()
    }

    private fun ensureVpnInterface() {
        if (vpnInterface != null) {
            return
        }

        val tunAddress4 = savedVpnIp ?: "10.0.0.2"
        val tunPrefixLen = savedVpnPrefixLen.coerceIn(1, 30)
        val tunMtu = if (isAdaptiveEnabled()) ADAPTIVE_TUN_MTU else savedVpnMtu.coerceAtLeast(576)

        // Build TUN (must stay in Kotlin — Android API).
        // setBlocking(false): Rust uses epoll/AsyncFd on the raw fd.
        // IPv6 is intentionally disabled in this client.
        val builder = Builder()
            .setSession("AIVPN")
            .addAddress(tunAddress4, tunPrefixLen)
            .addRoute("0.0.0.0", 0)
            .addDnsServer("8.8.8.8")
            .addDnsServer("1.1.1.1")
            .setMtu(tunMtu)
            .setBlocking(false)

        val allowedApps = SecureStorage.loadAllowedApps(this)
        for (pkg in allowedApps) {
            try {
                builder.addAllowedApplication(pkg)
            } catch (_: Exception) {
                // Package may have been uninstalled — skip silently
            }
        }

        // Domain-based split tunnel: resolve each excluded domain to IPv4 addresses at
        // connect time and add /32 exclusion routes so that traffic to those IPs bypasses
        // the VPN tunnel. This is the same approach used by NordVPN and ExpressVPN on
        // Android. Limitation: CDN IPs rotate and may differ per client, so exclusions
        // are best-effort. The domain list is re-resolved on every tunnel (re)connect.
        applyDomainExclusions(builder)

        vpnInterface = builder.establish() ?: throw Exception("Failed to establish VPN interface")
    }

    /**
     * Resolve each excluded domain to IPv4 addresses and register them as exclusion routes
     * via [Builder.excludeRoute] so traffic to those IPs bypasses the VPN tunnel.
     *
     * [Builder.excludeRoute] was added in API 33 (Android 13). On older devices the builder
     * has no exclusion-route primitive; we log a warning and skip — the stored domain list
     * will take effect once the device is on API 33+.
     *
     * Limitation: CDN IPs rotate and vary per client. Domains are re-resolved on every
     * tunnel (re)connect. DNS resolution runs synchronously here because
     * [ensureVpnInterface] is already called from a background coroutine (Dispatchers.IO).
     */
    private fun applyDomainExclusions(builder: Builder) {
        val domains = SecureStorage.loadExcludedDomains(this)
        if (domains.isEmpty()) return

        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
            Log.w(TAG, "Domain split tunnel requires API 33 (Android 13); " +
                "${domains.size} domain(s) configured but skipped on API ${Build.VERSION.SDK_INT}")
            return
        }

        val resolvedIps = mutableSetOf<Inet4Address>()

        for (domain in domains) {
            try {
                val addrs = InetAddress.getAllByName(domain)
                val v4addrs = addrs.filterIsInstance<Inet4Address>()
                if (v4addrs.isEmpty()) {
                    Log.d(TAG, "Domain $domain: no IPv4 addresses returned — skipping")
                } else {
                    Log.d(TAG, "Domain $domain resolved: ${v4addrs.map { it.hostAddress }}")
                    resolvedIps.addAll(v4addrs)
                }
            } catch (e: Exception) {
                Log.w(TAG, "DNS resolution failed for excluded domain $domain: ${e.message}")
            }
        }

        if (resolvedIps.isEmpty()) {
            Log.d(TAG, "No IPv4 addresses resolved for excluded domains")
            return
        }

        for (addr in resolvedIps) {
            try {
                // Builder.excludeRoute(IpPrefix) — API 33+. Traffic to this /32 prefix
                // is routed through the underlying network, not through the VPN tunnel.
                @Suppress("NewApi")
                builder.excludeRoute(android.net.IpPrefix(addr, 32))
                Log.d(TAG, "Exclusion route applied: ${addr.hostAddress}/32")
            } catch (e: Exception) {
                Log.w(TAG, "Failed to add exclusion route for ${addr.hostAddress}: ${e.message}")
            }
        }
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

    // ──────────── Profile-keyed connect ────────────

    /**
     * Load the VPN profile from EncryptedSharedPreferences by [profileId] and
     * start the tunnel.  Server keys stay in secure storage and are never
     * exposed as Intent extras.
     */
    private fun loadAndStartVpnFromProfile(profileId: String) {
        val profiles = SecureStorage.loadProfiles(this)
        val profile = profiles.find { it.id == profileId } ?: profiles.firstOrNull()
        if (profile == null) {
            Log.w(TAG, "loadAndStartVpnFromProfile: no profile for id=$profileId")
            return
        }
        val parsed = parseConnectionKeyInService(profile.key) ?: return
        val mtlsCert: ByteArray? = profile.mtlsCertBase64?.let { b64 ->
            try { Base64.decode(b64, Base64.DEFAULT).takeIf { it.size == 104 } }
            catch (_: Exception) { null }
        }
        startVpn(
            serverAddr      = parsed.server,
            serverKeyBase64 = parsed.serverKey,
            pskBase64       = parsed.psk,
            vpnIp           = parsed.vpnIp,
            serverVpnIp     = parsed.serverVpnIp,
            vpnPrefixLen    = parsed.prefixLen,
            vpnMtu          = parsed.mtu,
            mtlsCert        = mtlsCert,
        )
    }

    private data class ParsedConnectionKey(
        val server: String,
        val serverKey: String,
        val psk: String?,
        val vpnIp: String,
        val serverVpnIp: String,
        val prefixLen: Int,
        val mtu: Int,
    )

    private fun parseConnectionKeyInService(raw: String): ParsedConnectionKey? {
        val payload = raw.trim().let {
            if (it.startsWith("aivpn://")) it.removePrefix("aivpn://") else it
        }
        return try {
            val bytes = Base64.decode(payload,
                Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP)
            val json = JSONObject(String(bytes, Charsets.UTF_8))
            val net = json.optJSONObject("n")
            val vpnIp = net?.optString("client_ip")?.takeIf { it.isNotBlank() }
                ?: json.getString("i")
            val serverVpnIp = net?.optString("server_vpn_ip")?.takeIf { it.isNotBlank() }
                ?: "10.0.0.1"
            ParsedConnectionKey(
                server      = json.getString("s"),
                serverKey   = json.getString("k"),
                psk         = json.optString("p").takeIf { it.isNotEmpty() },
                vpnIp       = vpnIp,
                serverVpnIp = serverVpnIp,
                prefixLen   = net?.optInt("prefix_len", LEGACY_PREFIX_LEN) ?: LEGACY_PREFIX_LEN,
                mtu         = net?.optInt("mtu", DEFAULT_TUN_MTU) ?: DEFAULT_TUN_MTU,
            )
        } catch (e: Exception) {
            Log.e(TAG, "parseConnectionKeyInService failed: ${e.message}")
            null
        }
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
