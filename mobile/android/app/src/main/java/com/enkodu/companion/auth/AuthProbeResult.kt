package com.enkodu.companion.auth

data class AuthProbeResult(
    val state: AuthConnectionState,
    val message: String
)
