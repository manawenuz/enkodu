package com.enkodu.companion.auth

import java.net.URI

object AuthConfigValidator {
    fun normalizeServerUrl(raw: String): String {
        val trimmed = raw.trim().trimEnd('/')
        if (trimmed.isBlank()) {
            return ""
        }
        return "$trimmed/"
    }

    fun isValidServerUrl(raw: String): Boolean {
        val normalized = normalizeServerUrl(raw)
        if (normalized.isBlank()) {
            return false
        }
        return try {
            val uri = URI(normalized)
            val scheme = uri.scheme?.lowercase()
            (scheme == "http" || scheme == "https") && !uri.host.isNullOrBlank()
        } catch (_: Exception) {
            false
        }
    }
}
