package com.enkodu.companion.auth

enum class AuthMode(val persistedValue: String, val requiresToken: Boolean) {
    CompanionToken("companion_token", true),
    None("none", false);

    companion object {
        fun fromPersisted(value: String?): AuthMode {
            return entries.firstOrNull { it.persistedValue == value } ?: CompanionToken
        }
    }
}
