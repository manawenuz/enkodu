package com.enkodu.companion.auth

import okhttp3.Call
import okhttp3.Connection
import okhttp3.Interceptor
import okhttp3.Protocol
import okhttp3.Request
import okhttp3.Response
import okhttp3.ResponseBody.Companion.toResponseBody
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import java.util.concurrent.TimeUnit

class AuthHeaderInterceptorTest {

    @Test
    fun injectsBearerTokenWhenAvailable() {
        val interceptor = AuthHeaderInterceptor(
            tokenProvider = object : AuthTokenProvider {
                override fun bearerToken(): String = "secret-token"
            }
        )
        val chain = RecordingChain(
            Request.Builder()
                .url("https://enkodu.example.com/status")
                .build()
        )

        interceptor.intercept(chain)

        assertEquals("Bearer secret-token", chain.proceededRequest.header("Authorization"))
    }

    @Test
    fun leavesRequestUnchangedWhenTokenMissing() {
        val interceptor = AuthHeaderInterceptor(
            tokenProvider = object : AuthTokenProvider {
                override fun bearerToken(): String? = null
            }
        )
        val chain = RecordingChain(
            Request.Builder()
                .url("https://enkodu.example.com/status")
                .build()
        )

        interceptor.intercept(chain)

        assertNull(chain.proceededRequest.header("Authorization"))
    }

    @Test
    fun preservesExistingAuthorizationHeader() {
        val interceptor = AuthHeaderInterceptor(
            tokenProvider = object : AuthTokenProvider {
                override fun bearerToken(): String = "new-token"
            }
        )
        val chain = RecordingChain(
            Request.Builder()
                .url("https://enkodu.example.com/status")
                .header("Authorization", "Bearer existing-token")
                .build()
        )

        interceptor.intercept(chain)

        assertEquals("Bearer existing-token", chain.proceededRequest.header("Authorization"))
    }

    private class RecordingChain(
        private val originalRequest: Request
    ) : Interceptor.Chain {
        lateinit var proceededRequest: Request

        override fun request(): Request = originalRequest

        override fun proceed(request: Request): Response {
            proceededRequest = request
            return Response.Builder()
                .request(request)
                .protocol(Protocol.HTTP_1_1)
                .code(200)
                .message("OK")
                .body("{}".toResponseBody())
                .build()
        }

        override fun connection(): Connection? = null

        override fun call(): Call {
            throw UnsupportedOperationException("Not needed for this test")
        }

        override fun connectTimeoutMillis(): Int = TimeUnit.SECONDS.toMillis(30).toInt()

        override fun withConnectTimeout(timeout: Int, unit: TimeUnit): Interceptor.Chain = this

        override fun readTimeoutMillis(): Int = TimeUnit.SECONDS.toMillis(30).toInt()

        override fun withReadTimeout(timeout: Int, unit: TimeUnit): Interceptor.Chain = this

        override fun writeTimeoutMillis(): Int = TimeUnit.SECONDS.toMillis(30).toInt()

        override fun withWriteTimeout(timeout: Int, unit: TimeUnit): Interceptor.Chain = this
    }
}
