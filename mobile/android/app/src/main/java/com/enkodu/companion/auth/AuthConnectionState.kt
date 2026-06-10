package com.enkodu.companion.auth

enum class AuthConnectionState {
    Unknown,
    NotConfigured,
    Checking,
    Connected,
    TokenRejected,
    ServerUnreachable,
    PermissionDenied,
}
