package com.enkodu.companion.auth

object AuthProbeResultMapper {
    fun fromHttpCode(code: Int): AuthProbeResult {
        return when (code) {
            in 200..299 -> AuthProbeResult(
                state = AuthConnectionState.Connected,
                message = "Connected. Companion token accepted."
            )
            401 -> AuthProbeResult(
                state = AuthConnectionState.TokenRejected,
                message = "Token rejected. Check the companion token and try again."
            )
            403 -> AuthProbeResult(
                state = AuthConnectionState.PermissionDenied,
                message = "Permission denied. This token can reach the server but cannot use companion endpoints."
            )
            else -> AuthProbeResult(
                state = AuthConnectionState.ServerUnreachable,
                message = "Server returned HTTP $code while probing /status."
            )
        }
    }
}
