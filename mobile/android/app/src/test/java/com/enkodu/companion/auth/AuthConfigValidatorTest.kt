package com.enkodu.companion.auth

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AuthConfigValidatorTest {

    @Test
    fun normalizeServerUrl_trimsWhitespaceAndAppendsSlash() {
        assertEquals(
            "https://enkodu.example.com/",
            AuthConfigValidator.normalizeServerUrl("  https://enkodu.example.com  ")
        )
    }

    @Test
    fun isValidServerUrl_acceptsHttpAndHttpsHosts() {
        assertTrue(AuthConfigValidator.isValidServerUrl("https://enkodu.example.com"))
        assertTrue(AuthConfigValidator.isValidServerUrl("http://192.168.1.12:8090"))
    }

    @Test
    fun isValidServerUrl_rejectsInvalidValues() {
        assertFalse(AuthConfigValidator.isValidServerUrl(""))
        assertFalse(AuthConfigValidator.isValidServerUrl("enkodu.example.com"))
        assertFalse(AuthConfigValidator.isValidServerUrl("https:///missing-host"))
    }
}
