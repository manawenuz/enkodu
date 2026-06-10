package com.enkodu.companion.auth

import android.content.Context
import android.content.SharedPreferences
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

class AuthStore(context: Context) : AuthTokenProvider {

    private val appContext = context.applicationContext
    private val authPrefs: SharedPreferences = createPreferences(appContext)
    private val _authConfig = MutableStateFlow(readConfig())
    val authConfig: StateFlow<AuthConfig> = _authConfig.asStateFlow()

    fun currentAuthConfig(): AuthConfig = _authConfig.value

    fun save(config: AuthConfig): AuthConfig {
        val normalized = config.normalized()
        authPrefs.edit()
            .putString(KEY_SERVER_URL, normalized.serverUrl)
            .putString(KEY_AUTH_MODE, normalized.authMode.persistedValue)
            .putString(KEY_COMPANION_TOKEN, normalized.companionToken)
            .apply()
        _authConfig.value = normalized
        return normalized
    }

    fun clearToken() {
        save(currentAuthConfig().copy(companionToken = ""))
    }

    override fun bearerToken(): String? {
        val config = currentAuthConfig()
        if (!config.authMode.requiresToken) {
            return null
        }
        return config.companionToken.takeIf { it.isNotBlank() }
    }

    private fun readConfig(): AuthConfig {
        return AuthConfig(
            serverUrl = authPrefs.getString(KEY_SERVER_URL, "") ?: "",
            authMode = AuthMode.fromPersisted(authPrefs.getString(KEY_AUTH_MODE, null)),
            companionToken = authPrefs.getString(KEY_COMPANION_TOKEN, "") ?: ""
        ).normalized()
    }

    private fun createPreferences(context: Context): SharedPreferences {
        return try {
            val masterKey = MasterKey.Builder(context)
                .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
                .build()
            EncryptedSharedPreferences.create(
                context,
                AUTH_PREFS_NAME,
                masterKey,
                EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
                EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM
            )
        } catch (_: Exception) {
            context.getSharedPreferences(AUTH_PREFS_NAME_FALLBACK, Context.MODE_PRIVATE)
        }
    }

    companion object {
        private const val AUTH_PREFS_NAME = "enkodu_auth_secure"
        private const val AUTH_PREFS_NAME_FALLBACK = "enkodu_auth_fallback"
        private const val KEY_SERVER_URL = "server_url"
        private const val KEY_AUTH_MODE = "auth_mode"
        private const val KEY_COMPANION_TOKEN = "companion_token"
    }
}
