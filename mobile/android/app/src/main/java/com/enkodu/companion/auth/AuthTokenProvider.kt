package com.enkodu.companion.auth

interface AuthTokenProvider {
    fun bearerToken(): String?
}
