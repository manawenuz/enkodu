package com.enkodu.companion.auth

import okhttp3.Interceptor
import okhttp3.Response

class AuthHeaderInterceptor(
    private val tokenProvider: AuthTokenProvider
) : Interceptor {
    override fun intercept(chain: Interceptor.Chain): Response {
        val request = chain.request()
        if (request.header("Authorization") != null) {
            return chain.proceed(request)
        }

        val token = tokenProvider.bearerToken()
        if (token.isNullOrBlank()) {
            return chain.proceed(request)
        }

        return chain.proceed(
            request.newBuilder()
                .header("Authorization", "Bearer $token")
                .build()
        )
    }
}
