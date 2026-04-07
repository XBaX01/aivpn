package com.aivpn.client

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.net.VpnService
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.View
import android.widget.EditText
import android.widget.ImageButton
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.appcompat.app.AppCompatDelegate
import androidx.core.os.LocaleListCompat
import com.aivpn.client.databinding.ActivityMainBinding
import org.json.JSONObject
import java.util.UUID

/**
 * Main screen — server address, public key, connect/disconnect button,
 * connection timer, traffic stats, and EN/RU language toggle.
 *
 * v0.3.0: Uses EncryptedSharedPreferences for secure key storage.
 */
class MainActivity : AppCompatActivity() {

    private lateinit var binding: ActivityMainBinding
    private var isConnected = false

    private var profiles = mutableListOf<SecureStorage.ConnectionProfile>()
    private var activeProfileId: String? = null

    private val vpnPermissionLauncher = registerForActivityResult(
        ActivityResultContracts.StartActivityForResult()
    ) { result ->
        if (result.resultCode == Activity.RESULT_OK) {
            startVpnService()
        } else {
            Toast.makeText(this, getString(R.string.error_vpn_denied), Toast.LENGTH_SHORT).show()
        }
    }

    // Connection timer
    private val timerHandler = Handler(Looper.getMainLooper())
    private var connectionStartTime = 0L
    private val timerRunnable = object : Runnable {
        override fun run() {
            if (isConnected && connectionStartTime > 0) {
                val elapsed = (System.currentTimeMillis() - connectionStartTime) / 1000
                val h = elapsed / 3600
                val m = (elapsed % 3600) / 60
                val s = elapsed % 60
                binding.textTimer.text = String.format("%02d:%02d:%02d", h, m, s)
                binding.textDuration.text = String.format("%02d:%02d", h * 60 + m, s)
                timerHandler.postDelayed(this, 1000)
            }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        binding = ActivityMainBinding.inflate(layoutInflater)
        setContentView(binding.root)

        // Migrate legacy single connection key to profiles
        migrateLegacyKey()

        // Load profiles
        profiles = SecureStorage.loadProfiles(this)
        activeProfileId = SecureStorage.loadActiveProfileId(this)

        // If we have an active profile, load its key into the field
        val active = profiles.find { it.id == activeProfileId }
        if (active != null) {
            binding.editConnectionKey.setText(active.key)
        } else if (profiles.isNotEmpty()) {
            activeProfileId = profiles[0].id
            binding.editConnectionKey.setText(profiles[0].key)
            SecureStorage.saveActiveProfileId(this, profiles[0].id)
        } else {
            // Fallback: try legacy key
            binding.editConnectionKey.setText(SecureStorage.loadConnectionKey(this))
        }

        renderProfiles()

        // Update language button label
        updateLanguageButton()

        binding.btnConnect.setOnClickListener {
            if (isConnected) disconnect() else connect()
        }

        binding.btnLanguage.setOnClickListener {
            toggleLanguage()
        }

        binding.btnAddProfile.setOnClickListener {
            showProfileDialog(null)
        }

        binding.btnSplitTunnel.setOnClickListener {
            startActivity(Intent(this, SplitTunnelActivity::class.java))
        }

        updateSplitTunnelHint()

        // Restore connection state if service is already running
        if (AivpnService.isRunning) {
            isConnected = true
            updateUI(true, AivpnService.lastStatusText)
        }
    }

    // ──────────── Profile management ────────────

    private fun migrateLegacyKey() {
        val legacyKey = SecureStorage.loadConnectionKey(this)
        if (legacyKey.isNotEmpty()) {
            val existing = SecureStorage.loadProfiles(this)
            if (existing.none { it.key == legacyKey }) {
                val profile = SecureStorage.ConnectionProfile(
                    id = UUID.randomUUID().toString(),
                    name = extractServerName(legacyKey),
                    key = legacyKey
                )
                val updated = existing.toMutableList()
                updated.add(profile)
                SecureStorage.saveProfiles(this, updated)
                SecureStorage.saveActiveProfileId(this, profile.id)
            }
            SecureStorage.remove(this, "connection_key")
        }
    }

    private fun extractServerName(connectionKey: String): String {
        val parsed = parseConnectionKey(connectionKey) ?: return "Server"
        val server = parsed[0]
        val host = server.substringBefore(":")
        return host
    }

    private fun renderProfiles() {
        val container = binding.profileList
        container.removeAllViews()

        if (profiles.isEmpty()) {
            val empty = TextView(this).apply {
                text = getString(R.string.no_profiles)
                setTextColor(getColor(R.color.text_secondary))
                textSize = 13f
                setPadding(0, 8.dp, 0, 8.dp)
            }
            container.addView(empty)
            return
        }

        for (profile in profiles) {
            val row = LinearLayout(this).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = android.view.Gravity.CENTER_VERTICAL
                setPadding(0, 6.dp, 0, 6.dp)
                layoutParams = LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.MATCH_PARENT,
                    LinearLayout.LayoutParams.WRAP_CONTENT
                )
            }

            val isActive = profile.id == activeProfileId

            // Profile name + server info
            val nameView = TextView(this).apply {
                text = profile.name
                textSize = 14f
                setTextColor(getColor(if (isActive) R.color.accent else R.color.text_primary))
                if (isActive) setTypeface(null, android.graphics.Typeface.BOLD)
                layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f)
            }

            val editBtn = ImageButton(this).apply {
                setImageResource(android.R.drawable.ic_menu_edit)
                setBackgroundColor(android.graphics.Color.TRANSPARENT)
                setPadding(8.dp, 4.dp, 8.dp, 4.dp)
                contentDescription = getString(R.string.btn_edit)
                setOnClickListener { showProfileDialog(profile) }
            }

            val deleteBtn = ImageButton(this).apply {
                setImageResource(android.R.drawable.ic_menu_delete)
                setBackgroundColor(android.graphics.Color.TRANSPARENT)
                setPadding(8.dp, 4.dp, 8.dp, 4.dp)
                contentDescription = getString(R.string.btn_delete)
                setOnClickListener { confirmDeleteProfile(profile) }
            }

            // Tap the row to select
            row.setOnClickListener {
                if (isConnected) return@setOnClickListener
                activeProfileId = profile.id
                SecureStorage.saveActiveProfileId(this, profile.id)
                binding.editConnectionKey.setText(profile.key)
                renderProfiles()
            }

            row.addView(nameView)
            if (!isConnected) {
                row.addView(editBtn)
                row.addView(deleteBtn)
            }
            container.addView(row)
        }
    }

    private fun showProfileDialog(existing: SecureStorage.ConnectionProfile?) {
        if (isConnected) return

        // Use the dialog's theme context so EditText fields inherit proper colours
        // (white text, grey hints) instead of defaulting to the dark-on-dark activity theme.
        val dialogCtx = android.view.ContextThemeWrapper(this, R.style.Theme_AIVPN_Dialog)

        val layout = LinearLayout(dialogCtx).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(24.dp, 16.dp, 24.dp, 0)
        }

        val nameInput = EditText(dialogCtx).apply {
            hint = getString(R.string.hint_profile_name)
            setText(existing?.name ?: "")
            setSingleLine(true)
        }
        val keyInput = EditText(dialogCtx).apply {
            hint = getString(R.string.hint_profile_key)
            setText(existing?.key ?: binding.editConnectionKey.text.toString())
            setSingleLine(true)
            textSize = 13f
        }

        layout.addView(nameInput)
        layout.addView(keyInput)

        val title = if (existing != null)
            getString(R.string.dialog_edit_profile)
        else
            getString(R.string.dialog_add_profile)

        AlertDialog.Builder(this, R.style.Theme_AIVPN_Dialog)
            .setTitle(title)
            .setView(layout)
            .setPositiveButton(getString(R.string.btn_save)) { _, _ ->
                val name = nameInput.text.toString().trim()
                val key = keyInput.text.toString().trim()

                if (name.isEmpty()) {
                    Toast.makeText(this, getString(R.string.error_profile_name_empty), Toast.LENGTH_SHORT).show()
                    return@setPositiveButton
                }
                if (key.isEmpty()) {
                    Toast.makeText(this, getString(R.string.error_profile_key_empty), Toast.LENGTH_SHORT).show()
                    return@setPositiveButton
                }
                if (parseConnectionKey(key) == null) {
                    Toast.makeText(this, getString(R.string.error_profile_key_invalid), Toast.LENGTH_SHORT).show()
                    return@setPositiveButton
                }

                if (existing != null) {
                    val idx = profiles.indexOfFirst { it.id == existing.id }
                    if (idx >= 0) {
                        profiles[idx] = existing.copy(name = name, key = key)
                    }
                } else {
                    val newProfile = SecureStorage.ConnectionProfile(
                        id = UUID.randomUUID().toString(),
                        name = name,
                        key = key
                    )
                    profiles.add(newProfile)
                    activeProfileId = newProfile.id
                    SecureStorage.saveActiveProfileId(this, newProfile.id)
                    binding.editConnectionKey.setText(key)
                }
                SecureStorage.saveProfiles(this, profiles)
                renderProfiles()
            }
            .setNegativeButton(getString(R.string.btn_cancel), null)
            .show()
    }

    private fun confirmDeleteProfile(profile: SecureStorage.ConnectionProfile) {
        if (isConnected) return
        AlertDialog.Builder(this, R.style.Theme_AIVPN_Dialog)
            .setMessage(getString(R.string.confirm_delete_profile, profile.name))
            .setPositiveButton(getString(R.string.btn_delete)) { _, _ ->
                profiles.removeAll { it.id == profile.id }
                if (activeProfileId == profile.id) {
                    activeProfileId = profiles.firstOrNull()?.id
                    activeProfileId?.let { SecureStorage.saveActiveProfileId(this, it) }
                    binding.editConnectionKey.setText(
                        profiles.firstOrNull()?.key ?: ""
                    )
                }
                SecureStorage.saveProfiles(this, profiles)
                renderProfiles()
            }
            .setNegativeButton(getString(R.string.btn_cancel), null)
            .show()
    }

    private val Int.dp: Int get() = (this * resources.displayMetrics.density).toInt()

    private fun updateSplitTunnelHint() {
        val appCount = SecureStorage.loadAllowedApps(this).size
        val siteCount = SecureStorage.loadExcludedDomains(this).size
        binding.textSplitTunnelHint.text = when {
            appCount > 0 && siteCount > 0 -> getString(R.string.split_tunnel_hint_combined,
                getString(R.string.split_tunnel_hint_apps, appCount),
                getString(R.string.split_tunnel_hint_sites, siteCount))
            appCount > 0 -> getString(R.string.split_tunnel_vpn_count, appCount)
            siteCount > 0 -> getString(R.string.split_tunnel_hint_sites, siteCount) + " " + getString(R.string.split_tunnel_bypass_count, siteCount).substringAfter(" ")
            else -> getString(R.string.split_tunnel_none)
        }
    }

    override fun onResume() {
        super.onResume()
        // Register callbacks when activity becomes visible.
        // Using onResume/onPause instead of onCreate/onDestroy prevents the race condition
        // where a destroyed (rotated) Activity nullifies callbacks registered by the new one.
        AivpnService.statusCallback = { connected, statusText ->
            runOnUiThread {
                isConnected = connected
                updateUI(connected, statusText)
            }
        }

        AivpnService.trafficCallback = { uploadBytes, downloadBytes ->
            runOnUiThread {
                binding.textUpload.text = formatBytes(uploadBytes)
                binding.textDownload.text = formatBytes(downloadBytes)
            }
        }

        // Restore UI state if service is already running (e.g. after returning from
        // VPN permission dialog or screen rotation)
        if (AivpnService.isRunning) {
            isConnected = true
            updateUI(true, AivpnService.lastStatusText)
        }

        updateSplitTunnelHint()
    }

    override fun onPause() {
        super.onPause()
        // Unregister callbacks when activity is no longer in foreground.
        // Only nullify if activity is actually finishing (not just pausing for
        // VPN permission dialog, multi-window, etc.)
        if (isFinishing) {
            AivpnService.statusCallback = null
            AivpnService.trafficCallback = null
        }
    }

    /**
     * Parse connection key: aivpn://BASE64URL({"s":"host:port","k":"...","p":"...","i":"..."})
     * Returns (server, serverKey, psk, vpnIp) or null on failure.
     */
    private fun parseConnectionKey(key: String): Array<String>? {
        val raw = key.trim()
        val payload = if (raw.startsWith("aivpn://")) raw.removePrefix("aivpn://") else raw
        return try {
            // Decode URL-safe base64 (no padding)
            val jsonBytes = android.util.Base64.decode(payload,
                android.util.Base64.URL_SAFE or android.util.Base64.NO_PADDING or android.util.Base64.NO_WRAP)
            val json = JSONObject(String(jsonBytes))
            val server = json.getString("s")
            val serverKey = json.getString("k")
            val psk = json.getString("p")
            val vpnIp = json.getString("i")
            arrayOf(server, serverKey, psk, vpnIp)
        } catch (_: Exception) {
            null
        }
    }

    private fun connect() {
        val connectionKey = binding.editConnectionKey.text.toString().trim()
        if (connectionKey.isEmpty()) {
            Toast.makeText(this, getString(R.string.error_fill_fields), Toast.LENGTH_SHORT).show()
            return
        }

        val parsed = parseConnectionKey(connectionKey)
        if (parsed == null) {
            Toast.makeText(this, getString(R.string.error_invalid_connection_key), Toast.LENGTH_SHORT).show()
            return
        }

        // Auto-save if the key isn't already in profiles
        if (profiles.none { it.key == connectionKey }) {
            val profile = SecureStorage.ConnectionProfile(
                id = UUID.randomUUID().toString(),
                name = extractServerName(connectionKey),
                key = connectionKey
            )
            profiles.add(profile)
            activeProfileId = profile.id
            SecureStorage.saveProfiles(this, profiles)
            SecureStorage.saveActiveProfileId(this, profile.id)
            renderProfiles()
        }

        // Request VPN permission from the system
        val intent = VpnService.prepare(this)
        if (intent != null) {
            vpnPermissionLauncher.launch(intent)
        } else {
            startVpnService()
        }
    }

    private fun disconnect() {
        val intent = Intent(this, AivpnService::class.java).apply {
            action = AivpnService.ACTION_DISCONNECT
        }
        startService(intent)
    }

    private fun startVpnService() {
        val connectionKey = binding.editConnectionKey.text.toString().trim()
        val parsed = parseConnectionKey(connectionKey) ?: return
        val (server, serverKey, psk, vpnIp) = parsed

        val intent = Intent(this, AivpnService::class.java).apply {
            action = AivpnService.ACTION_CONNECT
            putExtra("server", server)
            putExtra("server_key", serverKey)
            putExtra("psk", psk)
            putExtra("vpn_ip", vpnIp)
        }
        startForegroundService(intent)
        updateUI(true, getString(R.string.status_connecting))
    }

    private fun updateUI(connected: Boolean, statusText: String) {
        isConnected = connected
        binding.btnConnect.text = getString(
            if (connected) R.string.btn_disconnect else R.string.btn_connect
        )
        binding.btnConnect.setBackgroundColor(
            getColor(if (connected) R.color.disconnect else R.color.accent)
        )
        binding.textStatus.text = statusText
        binding.statusDot.setBackgroundResource(
            if (connected) R.drawable.dot_green else R.drawable.dot_grey
        )

        // Show/hide stats and timer
        val statsVisibility = if (connected) View.VISIBLE else View.GONE
        binding.textTimer.visibility = statsVisibility
        binding.statsRow.visibility = statsVisibility

        // Lock/unlock input fields while connected
        binding.editConnectionKey.isEnabled = !connected
        binding.btnAddProfile.isEnabled = !connected
        renderProfiles()

        // Timer management
        if (connected && connectionStartTime == 0L) {
            connectionStartTime = System.currentTimeMillis()
            timerHandler.post(timerRunnable)
        } else if (!connected) {
            connectionStartTime = 0L
            timerHandler.removeCallbacks(timerRunnable)
            binding.textTimer.text = "00:00:00"
            binding.textUpload.text = "0 B"
            binding.textDownload.text = "0 B"
            binding.textDuration.text = "00:00"
        }
    }

    private fun toggleLanguage() {
        val currentLang = SecureStorage.loadLanguage(this)
        val newLang = if (currentLang == "en") "ru" else "en"

        SecureStorage.saveLanguage(this, newLang)

        val localeList = LocaleListCompat.forLanguageTags(newLang)
        AppCompatDelegate.setApplicationLocales(localeList)
    }

    private fun updateLanguageButton() {
        // Apply saved language on startup
        val savedLang = SecureStorage.loadLanguage(this)
        if (savedLang != "en") {
            val localeList = LocaleListCompat.forLanguageTags(savedLang)
            AppCompatDelegate.setApplicationLocales(localeList)
        }

        val currentLang = savedLang.uppercase()
        binding.btnLanguage.text = if (currentLang == "EN") "EN → RU" else "RU → EN"
    }

    private fun formatBytes(bytes: Long): String {
        return when {
            bytes < 1024 -> "$bytes B"
            bytes < 1024 * 1024 -> String.format("%.1f KB", bytes / 1024.0)
            bytes < 1024 * 1024 * 1024 -> String.format("%.1f MB", bytes / (1024.0 * 1024.0))
            else -> String.format("%.2f GB", bytes / (1024.0 * 1024.0 * 1024.0))
        }
    }

    override fun onDestroy() {
        timerHandler.removeCallbacks(timerRunnable)
        super.onDestroy()
    }
}
