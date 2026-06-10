package com.enkodu.companion

import android.content.Context
import android.content.SharedPreferences
import com.enkodu.companion.auth.AuthConfigValidator
import com.enkodu.companion.auth.AuthStore

class SettingsStore(context: Context) {

    private val appContext = context.applicationContext
    private val prefs: SharedPreferences = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
    private val authStore = AuthStore(appContext)

    var serverUrl: String
        get() = authStore.currentAuthConfig().serverUrl
        set(value) {
            authStore.save(authStore.currentAuthConfig().copy(serverUrl = value))
        }

    var displayName: String
        get() = prefs.getString(KEY_DISPLAY_NAME, "") ?: ""
        set(value) = prefs.edit().putString(KEY_DISPLAY_NAME, value).apply()

    var wifiOnlyUploads: Boolean
        get() = prefs.getBoolean(KEY_WIFI_ONLY_UPLOADS, true)
        set(value) = prefs.edit().putBoolean(KEY_WIFI_ONLY_UPLOADS, value).apply()

    var wifiOnlyDownloads: Boolean
        get() = prefs.getBoolean(KEY_WIFI_ONLY_DOWNLOADS, true)
        set(value) = prefs.edit().putBoolean(KEY_WIFI_ONLY_DOWNLOADS, value).apply()

    var maxUploadSizeMb: Int
        get() = prefs.getInt(KEY_MAX_UPLOAD_SIZE_MB, 100)
        set(value) = prefs.edit().putInt(KEY_MAX_UPLOAD_SIZE_MB, value).apply()

    var batteryMinPercent: Int
        get() = prefs.getInt(KEY_BATTERY_MIN_PERCENT, 15)
        set(value) = prefs.edit().putInt(KEY_BATTERY_MIN_PERCENT, value).apply()

    fun validateServerUrl(): Boolean {
        return AuthConfigValidator.isValidServerUrl(serverUrl)
    }

    fun validateAndShowError(context: Context): Boolean {
        if (!validateServerUrl()) {
            android.widget.Toast.makeText(context, "Please enter a valid server URL (https://...)", android.widget.Toast.LENGTH_LONG).show()
            return false
        }
        return true
    }

    companion object {
        private const val PREFS_NAME = "enkodu_settings"
        private const val KEY_DISPLAY_NAME = "display_name"
        private const val KEY_WIFI_ONLY_UPLOADS = "wifi_only_uploads"
        private const val KEY_WIFI_ONLY_DOWNLOADS = "wifi_only_downloads"
        private const val KEY_MAX_UPLOAD_SIZE_MB = "max_upload_size_mb"
        private const val KEY_BATTERY_MIN_PERCENT = "battery_min_percent"
    }
}
