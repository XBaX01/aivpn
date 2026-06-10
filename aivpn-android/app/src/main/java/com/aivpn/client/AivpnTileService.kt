package com.aivpn.client

import android.content.Intent
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import android.util.Base64
import android.util.Log
import org.json.JSONObject

/**
 * Quick Settings tile (notification shade shortcut) for toggling the VPN.
 * Requires Android 7+ (API 24). Registered in AndroidManifest.xml with the
 * BIND_QUICK_SETTINGS_TILE permission so only the system can bind to it.
 *
 * State sync: onStartListening() mirrors AivpnService.isRunning each time the
 * shade is opened. The tile shows ACTIVE (blue) while connected and INACTIVE
 * otherwise.
 *
 * Connect flow: loads the active profile from SecureStorage, parses the
 * connection key, then fires AivpnService.ACTION_CONNECT. If VPN permission
 * has not been granted yet, opens MainActivity to let the user grant it.
 */
class AivpnTileService : TileService() {

    companion object {
        private const val TAG = "AivpnTileService"
    }

    override fun onStartListening() {
        super.onStartListening()
        syncTileState()
    }

    override fun onClick() {
        super.onClick()
        if (AivpnService.isRunning) {
            disconnectVpn()
        } else {
            connectVpn()
        }
    }

    // ──────────── Private helpers ────────────

    private fun syncTileState() {
        val tile = qsTile ?: return
        if (AivpnService.isRunning) {
            tile.state = Tile.STATE_ACTIVE
            tile.contentDescription = getString(R.string.status_connected, getString(R.string.app_name))
        } else {
            tile.state = Tile.STATE_INACTIVE
            tile.contentDescription = getString(R.string.status_disconnected)
        }
        tile.updateTile()
    }

    private fun disconnectVpn() {
        val intent = Intent(this, AivpnService::class.java).apply {
            action = AivpnService.ACTION_DISCONNECT
        }
        startForegroundService(intent)
        qsTile?.let { tile ->
            tile.state = Tile.STATE_INACTIVE
            tile.updateTile()
        }
    }

    private fun connectVpn() {
        // If the app has not been granted VPN permission yet, the service cannot
        // start. Open MainActivity so the user can grant it through the normal flow.
        val vpnPermissionIntent = android.net.VpnService.prepare(this)
        if (vpnPermissionIntent != null) {
            val main = Intent(this, MainActivity::class.java).apply {
                flags = Intent.FLAG_ACTIVITY_NEW_TASK
            }
            startActivityAndCollapse(main)
            return
        }

        val profileId = SecureStorage.loadActiveProfileId(this)
        val profile = SecureStorage.loadProfiles(this)
            .let { list -> list.find { it.id == profileId } ?: list.firstOrNull() }

        if (profile == null) {
            Log.w(TAG, "No profile configured — opening MainActivity")
            qsTile?.let { it.state = Tile.STATE_UNAVAILABLE; it.updateTile() }
            val main = Intent(this, MainActivity::class.java).apply {
                flags = Intent.FLAG_ACTIVITY_NEW_TASK
            }
            startActivityAndCollapse(main)
            return
        }

        val parsed = parseKey(profile.key)
        if (parsed == null) {
            Log.w(TAG, "Failed to parse connection key for profile '${profile.name}'")
            return
        }

        val intent = Intent(this, AivpnService::class.java).apply {
            action = AivpnService.ACTION_CONNECT
            putExtra("server", parsed.server)
            putExtra("server_key", parsed.serverKey)
            putExtra("psk", parsed.psk)
            putExtra("vpn_ip", parsed.vpnIp)
            putExtra("prefix_len", parsed.prefixLen)
        }
        startForegroundService(intent)
        qsTile?.let { it.state = Tile.STATE_ACTIVE; it.updateTile() }
    }

    // Minimal connection key parser. The canonical parser is in MainActivity.parseConnectionKey;
    // keep field names in sync if the key format changes.
    private data class ParsedKey(
        val server: String,
        val serverKey: String,
        val psk: String?,
        val vpnIp: String,
        val prefixLen: Int,
    )

    private fun parseKey(raw: String): ParsedKey? {
        return try {
            val stripped = raw.trim().removePrefix("aivpn://")
            val bytes = Base64.decode(stripped, Base64.URL_SAFE or Base64.NO_PADDING)
            val json = JSONObject(String(bytes, Charsets.UTF_8))
            ParsedKey(
                server    = json.getString("s"),
                serverKey = json.getString("k"),
                psk       = json.optString("p").takeIf { it.isNotEmpty() },
                vpnIp     = json.getString("i"),
                prefixLen = json.optInt("x", 24),
            )
        } catch (e: Exception) {
            Log.e(TAG, "parseKey failed: ${e.message}")
            null
        }
    }
}
