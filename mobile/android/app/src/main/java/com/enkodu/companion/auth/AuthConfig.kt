package com.enkodu.companion.auth

data class AuthConfig(
    val serverUrl: String = "",
    val authMode: AuthMode = AuthMode.CompanionToken,
    val companionToken: String = ""
) {
    fun normalized(): AuthConfig {
        return copy(
            serverUrl = AuthConfigValidator.normalizeServerUrl(serverUrl),
            companionToken = companionToken.trim()
        )
    }

    fun isConfigured(): Boolean {
        val normalized = normalized()
        if (!AuthConfigValidator.isValidServerUrl(normalized.serverUrl)) {
            return false
        }
        return !normalized.authMode.requiresToken || normalized.companionToken.isNotBlank()
    }
}
