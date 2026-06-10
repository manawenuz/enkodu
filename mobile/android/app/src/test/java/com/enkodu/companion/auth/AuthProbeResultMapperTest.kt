package com.enkodu.companion.auth

import org.junit.Assert.assertEquals
import org.junit.Test

class AuthProbeResultMapperTest {

    @Test
    fun mapsSuccessResponseToConnected() {
        assertEquals(
            AuthConnectionState.Connected,
            AuthProbeResultMapper.fromHttpCode(200).state
        )
    }

    @Test
    fun mapsUnauthorizedToTokenRejected() {
        assertEquals(
            AuthConnectionState.TokenRejected,
            AuthProbeResultMapper.fromHttpCode(401).state
        )
    }

    @Test
    fun mapsForbiddenToPermissionDenied() {
        assertEquals(
            AuthConnectionState.PermissionDenied,
            AuthProbeResultMapper.fromHttpCode(403).state
        )
    }

    @Test
    fun mapsOtherHttpFailuresToServerUnreachable() {
        assertEquals(
            AuthConnectionState.ServerUnreachable,
            AuthProbeResultMapper.fromHttpCode(502).state
        )
    }
}
