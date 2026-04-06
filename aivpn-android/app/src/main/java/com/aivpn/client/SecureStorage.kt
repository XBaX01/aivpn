package com.aivpn.client

import android.content.Context
import android.content.SharedPreferences
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import org.json.JSONArray
import org.json.JSONObject

/**
 * Secure storage using EncryptedSharedPreferences.
 * Keys are encrypted with Android Keystore — safe from root access.
 */
object SecureStorage {

    private const val PREFS_FILE = "aivpn_secure_prefs"

    private fun getPrefs(context: Context): SharedPreferences {
        val masterKey = MasterKey.Builder(context)
            .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
            .build()

        return EncryptedSharedPreferences.create(
            context,
            PREFS_FILE,
            masterKey,
            EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
            EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM
        )
    }

    fun saveString(context: Context, key: String, value: String) {
        getPrefs(context).edit().putString(key, value).apply()
    }

    fun loadString(context: Context, key: String, defaultValue: String = ""): String {
        return try {
            getPrefs(context).getString(key, defaultValue) ?: defaultValue
        } catch (_: Exception) {
            defaultValue
        }
    }

    fun remove(context: Context, key: String) {
        getPrefs(context).edit().remove(key).apply()
    }

    // Connection key helpers (legacy single-key, kept for migration)
    fun saveConnectionKey(context: Context, key: String) {
        saveString(context, "connection_key", key)
    }

    fun loadConnectionKey(context: Context): String {
        return loadString(context, "connection_key")
    }

    // Language preference
    fun saveLanguage(context: Context, lang: String) {
        saveString(context, "language", lang)
    }

    fun loadLanguage(context: Context): String {
        return loadString(context, "language", "en")
    }

    // ──────────── Multi-profile management ────────────

    data class ConnectionProfile(
        val id: String,
        val name: String,
        val key: String
    )

    fun saveProfiles(context: Context, profiles: List<ConnectionProfile>) {
        val arr = JSONArray()
        for (p in profiles) {
            arr.put(JSONObject().apply {
                put("id", p.id)
                put("name", p.name)
                put("key", p.key)
            })
        }
        saveString(context, "profiles", arr.toString())
    }

    fun loadProfiles(context: Context): MutableList<ConnectionProfile> {
        val raw = loadString(context, "profiles")
        if (raw.isEmpty()) return mutableListOf()
        return try {
            val arr = JSONArray(raw)
            val result = mutableListOf<ConnectionProfile>()
            for (i in 0 until arr.length()) {
                val obj = arr.getJSONObject(i)
                result.add(ConnectionProfile(
                    id = obj.getString("id"),
                    name = obj.getString("name"),
                    key = obj.getString("key")
                ))
            }
            result
        } catch (_: Exception) {
            mutableListOf()
        }
    }

    fun saveActiveProfileId(context: Context, id: String) {
        saveString(context, "active_profile_id", id)
    }

    fun loadActiveProfileId(context: Context): String {
        return loadString(context, "active_profile_id")
    }

    // ──────────── Split tunneling ────────────

    private const val KEY_EXCLUDED_APPS = "split_tunnel_excluded_apps"

    fun saveExcludedApps(context: Context, packages: Set<String>) {
        val arr = JSONArray()
        for (pkg in packages) arr.put(pkg)
        saveString(context, KEY_EXCLUDED_APPS, arr.toString())
    }

    fun loadExcludedApps(context: Context): MutableSet<String> {
        val raw = loadString(context, KEY_EXCLUDED_APPS)
        if (raw.isEmpty()) return mutableSetOf()
        return try {
            val arr = JSONArray(raw)
            val result = mutableSetOf<String>()
            for (i in 0 until arr.length()) result.add(arr.getString(i))
            result
        } catch (_: Exception) {
            mutableSetOf()
        }
    }

    // ──────────── Excluded domains ────────────

    private const val KEY_EXCLUDED_DOMAINS = "split_tunnel_excluded_domains"

    fun saveExcludedDomains(context: Context, domains: List<String>) {
        val arr = JSONArray()
        for (d in domains) arr.put(d)
        saveString(context, KEY_EXCLUDED_DOMAINS, arr.toString())
    }

    fun loadExcludedDomains(context: Context): MutableList<String> {
        val raw = loadString(context, KEY_EXCLUDED_DOMAINS)
        if (raw.isEmpty()) return mutableListOf()
        return try {
            val arr = JSONArray(raw)
            val result = mutableListOf<String>()
            for (i in 0 until arr.length()) result.add(arr.getString(i))
            result
        } catch (_: Exception) {
            mutableListOf()
        }
    }
}
