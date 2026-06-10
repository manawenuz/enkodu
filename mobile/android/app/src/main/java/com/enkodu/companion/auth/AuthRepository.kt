package com.enkodu.companion.auth

import com.enkodu.companion.api.EnkoduApi
import java.io.IOException

class AuthRepository(
    private val authStore: AuthStore
) {
    fun createApi(serverUrl: String): EnkoduApi {
        return EnkoduApi.create(serverUrl, authStore)
    }

    suspend fun probe(config: AuthConfig): AuthProbeResult {
        val normalized = config.normalized()
        if (!AuthConfigValidator.isValidServerUrl(normalized.serverUrl)) {
            return AuthProbeResult(
                state = AuthConnectionState.NotConfigured,
                message = "Enter a valid server URL to continue."
            )
        }
        if (normalized.authMode.requiresToken && normalized.companionToken.isBlank()) {
            return AuthProbeResult(
                state = AuthConnectionState.NotConfigured,
                message = "Enter the companion token to continue."
            )
        }

        val provider = object : AuthTokenProvider {
            override fun bearerToken(): String? {
                return if (normalized.authMode.requiresToken) normalized.companionToken else null
            }
        }

        return try {
            val response = EnkoduApi.create(normalized.serverUrl, provider).getStatus()
            AuthProbeResultMapper.fromHttpCode(response.code())
        } catch (_: SecurityException) {
            AuthProbeResult(
                state = AuthConnectionState.PermissionDenied,
                message = "Permission denied while attempting the protected status probe."
            )
        } catch (error: IOException) {
            AuthProbeResult(
                state = AuthConnectionState.ServerUnreachable,
                message = "Server unreachable. Check the URL, VPN/LAN access, and try again."
            )
        } catch (error: Exception) {
            AuthProbeResult(
                state = AuthConnectionState.ServerUnreachable,
                message = error.message ?: "Server unreachable."
            )
        }
    }
}
